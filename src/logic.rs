//! Pure decision logic, kept apart from D-Bus and the WebSocket so it can be
//! unit-tested: settings parsing, which data a key needs, timing rules (call
//! expiry, album-art grace, volume settle, SMS arming) and output selection.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value as Json;

use crate::kdeconnect::{MediaState, NotificationInfo, Sink};
use crate::render::CallInfo;

pub const MEDIA_PLUGIN: &str = "kdeconnect_mprisremote";

/// How long to keep the previous cover up while the new track's cover arrives.
pub const ART_GRACE: Duration = Duration::from_secs(2);
/// After a volume change, trust our own value over polls until the device catches up.
pub const VOLUME_SETTLE: Duration = Duration::from_millis(1500);
/// SMS keys need a second press within this window.
pub const SMS_CONFIRM: Duration = Duration::from_secs(3);
/// telephony reports call events but not when a call ends, so states expire.
pub const RINGING_FOR: Duration = Duration::from_secs(45);
pub const TALKING_FOR: Duration = Duration::from_secs(30 * 60);

macro_rules! kinds {
	($($variant:ident => $short:literal, $plugin:literal;)*) => {
		/// Every action the plugin provides, by its UUID suffix.
		#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
		pub enum Kind { $($variant),* }

		impl Kind {
			pub const ALL: &'static [Kind] = &[$(Kind::$variant),*];

			/// The action UUID's last segment, e.g. "playpause".
			pub fn short(self) -> &'static str {
				match self { $(Kind::$variant => $short),* }
			}

			/// The KDE Connect plugin this action needs on the device.
			pub fn plugin(self) -> &'static str {
				match self { $(Kind::$variant => $plugin),* }
			}

			pub fn from_short(short: &str) -> Option<Kind> {
				match short { $($short => Some(Kind::$variant),)* _ => None }
			}
		}
	};
}

kinds! {
	PlayPause => "playpause", "kdeconnect_mprisremote";
	Next => "next", "kdeconnect_mprisremote";
	Previous => "previous", "kdeconnect_mprisremote";
	NowPlaying => "nowplaying", "kdeconnect_mprisremote";
	VolumeUp => "volumeup", "kdeconnect_mprisremote";
	VolumeDown => "volumedown", "kdeconnect_mprisremote";
	Mute => "mute", "kdeconnect_mprisremote";
	SeekBack => "seekback", "kdeconnect_mprisremote";
	SeekForward => "seekforward", "kdeconnect_mprisremote";
	SwitchPlayer => "switchplayer", "kdeconnect_mprisremote";
	Battery => "battery", "kdeconnect_battery";
	FindMyPhone => "findmyphone", "kdeconnect_findmyphone";
	Notifications => "notifications", "kdeconnect_notifications";
	Clipboard => "clipboard", "kdeconnect_clipboard";
	RemoteCommand => "remotecommand", "kdeconnect_remotecommands";
	Lock => "lock", "kdeconnect_lockdevice";
	PcVolumeUp => "pcvolumeup", "kdeconnect_remotesystemvolume";
	PcVolumeDown => "pcvolumedown", "kdeconnect_remotesystemvolume";
	PcMute => "pcmute", "kdeconnect_remotesystemvolume";
	Call => "call", "kdeconnect_telephony";
	Signal => "signal", "kdeconnect_connectivity_report";
	Sms => "sms", "kdeconnect_sms";
	Share => "share", "kdeconnect_share";
	Ping => "ping", "kdeconnect_ping";
}

impl Kind {
	/// Keys that repeat while held, and which way they step.
	pub fn hold_direction(self) -> Option<i32> {
		match self {
			Kind::VolumeUp | Kind::PcVolumeUp => Some(1),
			Kind::VolumeDown | Kind::PcVolumeDown => Some(-1),
			_ => None,
		}
	}

	pub fn is_pc_volume(self) -> bool {
		matches!(self, Kind::PcVolumeUp | Kind::PcVolumeDown | Kind::PcMute)
	}

	/// Short label for the "Not supported" footer.
	pub fn short_name(self) -> &'static str {
		match self {
			Kind::RemoteCommand => "Run command",
			Kind::Lock => "Lock",
			Kind::PcVolumeUp | Kind::PcVolumeDown => "PC volume",
			Kind::PcMute => "PC mute",
			Kind::Call => "Calls",
			Kind::Signal => "Signal",
			Kind::Sms => "SMS",
			Kind::Share => "Share",
			Kind::Ping => "Ping",
			_ => "",
		}
	}
}

/// Per-key settings, parsed leniently: the settings panel saves numbers as
/// numbers, but anything missing or malformed falls back to its default
/// instead of discarding every other setting.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
	/// Empty means "first reachable device that supports this key".
	pub device_id: String,
	/// Empty means whatever player KDE Connect has selected.
	pub player: String,
	pub show_art: bool,
	pub show_text: bool,
	pub press_action: String,
	pub volume_step: i64,
	pub seek_seconds: i64,
	pub include_silent: bool,
	pub command_key: String,
	/// Empty means the other computer's default output.
	pub sink: String,
	pub sms_destination: String,
	pub sms_message: String,
	pub sms_label: String,
	/// A URL opens on the device; anything else is shared as text.
	pub share_target: String,
	pub share_label: String,
	pub ping_message: String,
}

impl Default for Settings {
	fn default() -> Self {
		Self {
			device_id: String::new(),
			player: String::new(),
			show_art: true,
			show_text: true,
			press_action: "PlayPause".into(),
			volume_step: 5,
			seek_seconds: 10,
			include_silent: false,
			command_key: String::new(),
			sink: String::new(),
			sms_destination: String::new(),
			sms_message: String::new(),
			sms_label: String::new(),
			share_target: String::new(),
			share_label: String::new(),
			ping_message: String::new(),
		}
	}
}

impl Settings {
	pub fn from_json(json: &Json) -> Self {
		let defaults = Settings::default();
		let string = |key: &str, default: &str| match json.get(key) {
			Some(Json::String(s)) => s.clone(),
			Some(Json::Number(n)) => n.to_string(),
			_ => default.to_string(),
		};
		let boolean = |key: &str, default: bool| match json.get(key) {
			Some(Json::Bool(b)) => *b,
			Some(Json::String(s)) => s == "true",
			_ => default,
		};
		let number = |key: &str, default: i64| {
			let value = match json.get(key) {
				Some(Json::Number(n)) => n.as_f64(),
				Some(Json::String(s)) => s.trim().parse::<f64>().ok(),
				_ => None,
			};
			value
				.filter(|v| v.is_finite() && *v >= 1.0)
				.map(|v| v as i64)
				.unwrap_or(default)
		};
		Settings {
			device_id: string("deviceId", &defaults.device_id),
			player: string("player", &defaults.player),
			show_art: boolean("showArt", defaults.show_art),
			show_text: boolean("showText", defaults.show_text),
			press_action: string("pressAction", &defaults.press_action),
			volume_step: number("volumeStep", defaults.volume_step),
			seek_seconds: number("seekSeconds", defaults.seek_seconds),
			include_silent: boolean("includeSilent", defaults.include_silent),
			command_key: string("commandKey", ""),
			sink: string("sink", ""),
			sms_destination: string("smsDestination", ""),
			sms_message: string("smsMessage", ""),
			sms_label: string("smsLabel", ""),
			share_target: string("shareTarget", ""),
			share_label: string("shareLabel", ""),
			ping_message: string("pingMessage", ""),
		}
	}

	/// Key of the device lookup this key shares: the explicit device id, or
	/// `auto:<plugin>` so each feature can auto-pick a device that supports it.
	pub fn source_key(&self, kind: Kind) -> String {
		if self.device_id.is_empty() {
			format!("auto:{}", kind.plugin())
		} else {
			self.device_id.clone()
		}
	}
}

/// What one device lookup has to fetch for the keys that use it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Needs {
	pub media: bool,
	pub art: bool,
	pub battery: bool,
	pub notifications: bool,
	pub lock: bool,
	pub sinks: bool,
	pub connectivity: bool,
	pub commands: bool,
}

impl Needs {
	pub fn add(&mut self, kind: Kind, settings: &Settings) {
		self.media |= kind.plugin() == MEDIA_PLUGIN;
		self.art |= kind == Kind::NowPlaying && settings.show_art;
		self.battery |= kind == Kind::Battery;
		self.notifications |= kind == Kind::Notifications;
		self.lock |= kind == Kind::Lock;
		self.sinks |= kind.is_pc_volume();
		self.connectivity |= kind == Kind::Signal;
		self.commands |= kind == Kind::RemoteCommand;
	}
}

/// A call event as recorded from telephony's callReceived signal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallRecord {
	pub info: CallInfo,
	pub at: Instant,
}

/// The call to show, if it's still relevant: ringing turns into missed after
/// [`RINGING_FOR`], and "on call" clears after [`TALKING_FOR`].
pub fn current_call(record: Option<&CallRecord>, now: Instant) -> Option<CallInfo> {
	let record = record?;
	let age = now.saturating_duration_since(record.at);
	match record.info.event.as_str() {
		"ringing" if age > RINGING_FOR => Some(CallInfo {
			event: "missedCall".into(),
			..record.info.clone()
		}),
		"talking" if age > TALKING_FOR => None,
		_ => Some(record.info.clone()),
	}
}

/// Album art cached for the current track. `key` is empty while the next
/// track's cover is pending; `pending_since` is when that started.
#[derive(Clone, Debug, PartialEq)]
pub struct Art {
	pub key: String,
	pub path: PathBuf,
	pub album: String,
	pub uri: String,
	pub pending_since: Option<Instant>,
}

/// Identifies a track's cover: player, title and album.
pub fn art_key(media: &MediaState) -> String {
	format!("{}|{}|{}", media.player, media.title, media.album)
}

/// Whether the bridge's art path can be trusted for the current track. The
/// bridge updates the title ~300ms before the cover, so for a moment a new
/// track still points at the previous cover; the same file is only right if
/// the album didn't change.
pub fn accept_art_path(path: &Path, previous: Option<&Art>, album: &str) -> bool {
	!matches!(previous, Some(prev) if prev.path == path && prev.album != album)
}

/// Marks the cached art as pending (keeping the old cover for the grace window).
pub fn mark_art_pending(previous: Option<Art>, now: Instant) -> Option<Art> {
	previous.map(|art| Art {
		pending_since: if art.key.is_empty() {
			art.pending_since
		} else {
			Some(now)
		},
		key: String::new(),
		..art
	})
}

/// The cover to draw: the current one, or the previous one for up to
/// [`ART_GRACE`] while the new track's cover arrives.
pub fn visible_art(art: Option<&Art>, now: Instant) -> Option<&str> {
	let art = art?;
	let current = !art.key.is_empty()
		|| art
			.pending_since
			.is_some_and(|since| now.saturating_duration_since(since) < ART_GRACE);
	current.then_some(art.uri.as_str())
}

/// A volume we just set, preferred over polled values while the device catches up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolumeTarget {
	pub volume: i32,
	pub at: Instant,
}

pub fn settled_volume(polled: Option<i32>, target: Option<VolumeTarget>, now: Instant) -> Option<i32> {
	match target {
		Some(t) if now.saturating_duration_since(t.at) < VOLUME_SETTLE => Some(t.volume),
		_ => polled,
	}
}

/// Media volume after one step, clamped to 0-100.
pub fn stepped_volume(current: i32, direction: i32, step: i64) -> i32 {
	(i64::from(current) + i64::from(direction) * step).clamp(0, 100) as i32
}

/// Mute toggle: remember the level and go to 0, or restore it (50 if unknown).
pub fn toggled_mute(current: i32, muted_from: Option<i32>) -> (i32, Option<i32>) {
	if current > 0 {
		(0, Some(current))
	} else {
		(muted_from.filter(|v| *v > 0).unwrap_or(50), muted_from)
	}
}

/// The chosen output, else the other computer's default ("enabled"), else the first.
pub fn selected_sink<'a>(sinks: &'a [Sink], preferred: &str) -> Option<&'a Sink> {
	sinks
		.iter()
		.find(|s| !preferred.is_empty() && s.name == preferred)
		.or_else(|| sinks.iter().find(|s| s.enabled))
		.or_else(|| sinks.first())
}

fn max_volume(sink: &Sink) -> f64 {
	if sink.max_volume > 0.0 {
		sink.max_volume
	} else {
		65536.0
	}
}

pub fn sink_percent(sink: &Sink) -> i64 {
	(100.0 * sink.volume / max_volume(sink)).round() as i64
}

/// Raw output volume after one step of `step` percent of its maximum.
pub fn stepped_sink_volume(sink: &Sink, direction: i32, step: i64) -> f64 {
	let max = max_volume(sink);
	(sink.volume + f64::from(direction) * step as f64 * max / 100.0).clamp(0.0, max)
}

/// What a press of an SMS key does: arm it, or send (and disarm).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmsPress {
	Arm(Instant),
	Send,
}

pub fn sms_press(armed_until: Option<Instant>, now: Instant) -> SmsPress {
	match armed_until {
		Some(until) if until > now => SmsPress::Send,
		_ => SmsPress::Arm(now + SMS_CONFIRM),
	}
}

/// Count of notifications to show and the newest one's app. Ids are assigned
/// in posting order, so the highest is the newest.
pub fn visible_notifications(
	notifications: &HashMap<String, NotificationInfo>,
	include_silent: bool,
) -> (usize, String) {
	let mut visible: Vec<(&String, &NotificationInfo)> = notifications
		.iter()
		.filter(|(_, info)| include_silent || !info.silent)
		.collect();
	visible.sort_by_key(|(id, _)| std::cmp::Reverse(id.parse::<u64>().unwrap_or(0)));
	let newest = visible
		.first()
		.map(|(_, info)| info.app_name.clone())
		.unwrap_or_default();
	(visible.len(), newest)
}

/// Playback position now, extrapolated from the last poll while playing.
pub fn estimated_position(media: &MediaState, fetched_at: Instant, now: Instant) -> i64 {
	let elapsed = if media.is_playing {
		now.saturating_duration_since(fetched_at).as_millis() as i64
	} else {
		0
	};
	(media.position + elapsed).min(media.length.max(0))
}

/// Seek target in ms, kept a second short of the end.
pub fn seek_target(position: i64, length: i64, forward: bool, seconds: i64) -> i64 {
	let delta = seconds.max(1) * 1000;
	let end = if length > 0 { length - 1000 } else { i64::MAX };
	(position + if forward { delta } else { -delta }).min(end).max(0)
}

pub fn is_url(target: &str) -> bool {
	let Some((scheme, rest)) = target.split_once("://") else {
		return false;
	};
	!rest.is_empty()
		&& scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
		&& scheme
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	fn sink(name: &str, volume: f64, enabled: bool) -> Sink {
		Sink {
			name: name.into(),
			description: name.into(),
			volume,
			max_volume: 65536.0,
			muted: false,
			enabled,
		}
	}

	#[test]
	fn settings_parse_leniently() {
		let s = Settings::from_json(
			&json!({"deviceId": "abc", "volumeStep": "7", "seekSeconds": 0, "showArt": false, "bogus": 1}),
		);
		assert_eq!(s.device_id, "abc");
		assert_eq!(s.volume_step, 7);
		assert_eq!(s.seek_seconds, 10, "out-of-range numbers fall back to the default");
		assert!(!s.show_art);
		assert!(s.show_text);
		assert_eq!(Settings::from_json(&Json::Null), Settings::default());
	}

	#[test]
	fn auto_sources_are_per_feature() {
		let auto = Settings::default();
		assert_eq!(auto.source_key(Kind::Battery), "auto:kdeconnect_battery");
		assert_eq!(auto.source_key(Kind::Lock), "auto:kdeconnect_lockdevice");
		let pinned = Settings {
			device_id: "dev1".into(),
			..Settings::default()
		};
		assert_eq!(pinned.source_key(Kind::Lock), "dev1");
	}

	#[test]
	fn kinds_round_trip_and_cover_all_24_actions() {
		assert_eq!(Kind::ALL.len(), 24);
		for kind in Kind::ALL {
			assert_eq!(Kind::from_short(kind.short()), Some(*kind));
		}
	}

	#[test]
	fn needs_follow_visible_keys() {
		let mut needs = Needs::default();
		needs.add(Kind::NowPlaying, &Settings::default());
		needs.add(Kind::PcMute, &Settings::default());
		assert!(needs.media && needs.art && needs.sinks);
		assert!(!needs.battery && !needs.lock);
		let mut no_art = Needs::default();
		no_art.add(
			Kind::NowPlaying,
			&Settings {
				show_art: false,
				..Settings::default()
			},
		);
		assert!(no_art.media && !no_art.art);
	}

	#[test]
	fn ringing_becomes_missed_and_talking_expires() {
		let t0 = Instant::now();
		let ringing = CallRecord {
			info: CallInfo {
				event: "ringing".into(),
				number: "1".into(),
				name: "Sam".into(),
			},
			at: t0,
		};
		assert_eq!(
			current_call(Some(&ringing), t0 + Duration::from_secs(10))
				.unwrap()
				.event,
			"ringing"
		);
		assert_eq!(
			current_call(Some(&ringing), t0 + Duration::from_secs(46))
				.unwrap()
				.event,
			"missedCall"
		);
		let talking = CallRecord {
			info: CallInfo {
				event: "talking".into(),
				..ringing.info.clone()
			},
			at: t0,
		};
		assert!(current_call(Some(&talking), t0 + Duration::from_secs(60)).is_some());
		assert!(current_call(Some(&talking), t0 + Duration::from_secs(31 * 60)).is_none());
		let missed = CallRecord {
			info: CallInfo {
				event: "missedCall".into(),
				..ringing.info.clone()
			},
			at: t0,
		};
		assert!(
			current_call(Some(&missed), t0 + Duration::from_secs(3600)).is_some(),
			"missed stays until dismissed"
		);
		assert!(current_call(None, t0).is_none());
	}

	#[test]
	fn stale_cover_for_a_new_album_is_rejected() {
		let prev = Art {
			key: "k".into(),
			path: "/art/1.jpg".into(),
			album: "Damaged".into(),
			uri: "data:1".into(),
			pending_since: None,
		};
		assert!(
			!accept_art_path(Path::new("/art/1.jpg"), Some(&prev), "Betty"),
			"same file, new album: stale"
		);
		assert!(
			accept_art_path(Path::new("/art/1.jpg"), Some(&prev), "Damaged"),
			"same album shares a cover"
		);
		assert!(accept_art_path(Path::new("/art/2.jpg"), Some(&prev), "Betty"));
		assert!(accept_art_path(Path::new("/art/1.jpg"), None, "Betty"));
	}

	#[test]
	fn previous_cover_shows_for_the_grace_window_only() {
		let t0 = Instant::now();
		let art = Art {
			key: "k".into(),
			path: "/a.jpg".into(),
			album: "A".into(),
			uri: "data:a".into(),
			pending_since: None,
		};
		assert_eq!(visible_art(Some(&art), t0), Some("data:a"));
		let pending = mark_art_pending(Some(art), t0).unwrap();
		assert!(pending.key.is_empty());
		assert_eq!(
			visible_art(Some(&pending), t0 + Duration::from_millis(300)),
			Some("data:a")
		);
		assert_eq!(visible_art(Some(&pending), t0 + Duration::from_secs(5)), None);
		// Staying pending doesn't restart the grace window.
		let still = mark_art_pending(Some(pending.clone()), t0 + Duration::from_secs(1)).unwrap();
		assert_eq!(still.pending_since, pending.pending_since);
	}

	#[test]
	fn volume_settles_on_our_target_then_trusts_polls() {
		let t0 = Instant::now();
		let target = Some(VolumeTarget { volume: 70, at: t0 });
		assert_eq!(
			settled_volume(Some(60), target, t0 + Duration::from_millis(500)),
			Some(70)
		);
		assert_eq!(settled_volume(Some(60), target, t0 + Duration::from_secs(2)), Some(60));
		assert_eq!(stepped_volume(98, 1, 5), 100);
		assert_eq!(stepped_volume(3, -1, 5), 0);
	}

	#[test]
	fn mute_remembers_and_restores() {
		assert_eq!(toggled_mute(60, None), (0, Some(60)));
		assert_eq!(toggled_mute(0, Some(60)), (60, Some(60)));
		assert_eq!(toggled_mute(0, None), (50, None));
	}

	#[test]
	fn sink_selection_prefers_choice_then_default_then_first() {
		let sinks = vec![sink("hdmi", 32768.0, false), sink("speakers", 26214.0, true)];
		assert_eq!(selected_sink(&sinks, "hdmi").unwrap().name, "hdmi");
		assert_eq!(selected_sink(&sinks, "").unwrap().name, "speakers");
		assert_eq!(selected_sink(&sinks, "gone").unwrap().name, "speakers");
		assert_eq!(selected_sink(&[sink("only", 0.0, false)], "").unwrap().name, "only");
		assert!(selected_sink(&[], "").is_none());
		assert_eq!(sink_percent(&sinks[1]), 40);
		assert_eq!(stepped_sink_volume(&sinks[1], 1, 5).round(), 29491.0);
		assert_eq!(stepped_sink_volume(&sink("x", 65000.0, true), 1, 5), 65536.0);
	}

	#[test]
	fn sms_needs_a_second_press_in_time() {
		let t0 = Instant::now();
		let SmsPress::Arm(until) = sms_press(None, t0) else {
			panic!("first press must arm")
		};
		assert_eq!(sms_press(Some(until), t0 + Duration::from_secs(1)), SmsPress::Send);
		assert!(matches!(
			sms_press(Some(until), t0 + Duration::from_secs(4)),
			SmsPress::Arm(_)
		));
	}

	#[test]
	fn notifications_skip_silent_and_find_newest() {
		let mut map = HashMap::new();
		map.insert(
			"2".into(),
			NotificationInfo {
				app_name: "System UI".into(),
				silent: true,
			},
		);
		map.insert(
			"10".into(),
			NotificationInfo {
				app_name: "Signal".into(),
				silent: false,
			},
		);
		map.insert(
			"3".into(),
			NotificationInfo {
				app_name: "Mail".into(),
				silent: false,
			},
		);
		assert_eq!(visible_notifications(&map, false), (2, "Signal".into()));
		assert_eq!(visible_notifications(&map, true).0, 3);
		assert_eq!(visible_notifications(&HashMap::new(), false), (0, String::new()));
	}

	#[test]
	fn position_extrapolates_and_seek_clamps() {
		let t0 = Instant::now();
		let media = MediaState {
			position: 1000,
			length: 5000,
			is_playing: true,
			..Default::default()
		};
		assert_eq!(estimated_position(&media, t0, t0 + Duration::from_millis(500)), 1500);
		assert_eq!(estimated_position(&media, t0, t0 + Duration::from_secs(60)), 5000);
		assert_eq!(seek_target(62_800, 177_000, true, 10), 72_800);
		assert_eq!(seek_target(5_000, 177_000, false, 10), 0);
		assert_eq!(seek_target(170_000, 177_000, true, 10), 176_000);
	}

	#[test]
	fn url_detection_matches_the_share_rule() {
		assert!(is_url("https://kde.org"));
		assert!(is_url("tel+x://1"));
		assert!(!is_url("On my way"));
		assert!(!is_url("1http://x"));
	}
}
