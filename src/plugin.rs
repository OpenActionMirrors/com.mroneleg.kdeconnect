//! Plugin state and behaviour: which keys are visible, what each device lookup
//! has fetched, polling, rendering and key handling.
//!
//! State lives behind one std mutex that is never held across an `.await`;
//! D-Bus calls happen outside it and their results are written back after.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use serde_json::{Value as Json, json};
use tokio::task::JoinHandle;

use crate::kdeconnect::{
	self, Backend, Battery, Connectivity, Device, MediaState, NotificationInfo, RemoteCommand, Signal, Sink,
};
use crate::logic::{self, Art, CallRecord, Kind, MEDIA_PLUGIN, Needs, Settings, SmsPress, VolumeTarget};
use crate::render::{self, CallInfo, Glyph, NowPlaying};

pub const PREFIX: &str = "com.mroneleg.kdeconnect.";

const POLL_INTERVAL: Duration = Duration::from_secs(1);
const MARQUEE_INTERVAL: Duration = Duration::from_millis(200);
const MARQUEE_STEP_PX: f64 = 4.0;
const DEVICE_CACHE: Duration = Duration::from_secs(30);
const SIGNAL_DEBOUNCE: Duration = Duration::from_millis(100);
// Holding a volume key repeats after HOLD_DELAY, every HOLD_REPEAT.
const HOLD_DELAY: Duration = Duration::from_millis(400);
const HOLD_REPEAT: Duration = Duration::from_millis(150);
// Re-poll shortly after a press so the key reflects what the device did.
const AFTER_PRESS: Duration = Duration::from_millis(400);

/// One visible key.
struct Entry {
	kind: Kind,
	settings: Settings,
	last_image: Option<String>,
	marquee_offset: f64,
	armed_until: Option<Instant>,
	hold: Option<JoinHandle<()>>,
}

/// One device lookup, shared by every key with the same [`Settings::source_key`].
#[derive(Default)]
struct Source {
	device_id: String,
	plugin: Option<&'static str>,
	device: Option<Device>,
	resolved_at: Option<Instant>,
	media: Option<MediaState>,
	fetched_at: Option<Instant>,
	art: Option<Art>,
	battery: Option<Battery>,
	notifications: HashMap<String, NotificationInfo>,
	locked: Option<bool>,
	sinks: Vec<Sink>,
	sink_target: Option<(String, Sink, Instant)>,
	connectivity: Option<Connectivity>,
	commands: Vec<RemoteCommand>,
	volume_target: Option<VolumeTarget>,
	muted_from: Option<i32>,
	/// Serialises volume writes so held-key repeats apply in order.
	volume_lock: Arc<tokio::sync::Mutex<()>>,
}

impl Source {
	fn supports(&self, kind: Kind) -> bool {
		self.device
			.as_ref()
			.is_some_and(|d| d.plugins.iter().any(|p| p == kind.plugin()))
	}
}

#[derive(Default)]
struct State {
	entries: HashMap<String, Entry>,
	sources: HashMap<String, Source>,
	/// device id -> the newest telephony event.
	calls: HashMap<String, CallRecord>,
}

impl State {
	fn source_mut(&mut self, kind: Kind, settings: &Settings) -> &mut Source {
		self.sources.entry(settings.source_key(kind)).or_insert_with(|| Source {
			device_id: settings.device_id.clone(),
			plugin: settings.device_id.is_empty().then(|| kind.plugin()),
			..Source::default()
		})
	}
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Mutex::default);
static BACKEND: OnceLock<Arc<Backend>> = OnceLock::new();

fn state() -> std::sync::MutexGuard<'static, State> {
	STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn backend() -> Arc<Backend> {
	BACKEND.get().expect("backend initialised in start()").clone()
}

pub fn kind_of(action_uuid: &str) -> Option<Kind> {
	action_uuid.strip_prefix(PREFIX).and_then(Kind::from_short)
}

// ---------------------------------------------------------------- rendering

fn render_entry(entry: &Entry, source: &Source, calls: &HashMap<String, CallRecord>, now: Instant) -> Option<String> {
	// Keep the manifest's default image until the first lookup finishes.
	if source.device.is_none() && source.resolved_at.is_none() {
		return None;
	}
	let Some(device) = &source.device else {
		return Some(render::glyph_image(Glyph::Disconnected, render::RED, "No device"));
	};
	if !source.supports(entry.kind) {
		return Some(render::text_image(
			&device.name,
			"Not supported",
			entry.kind.short_name(),
		));
	}
	let settings = &entry.settings;
	let media = source.media.as_ref();
	let volume_label = || {
		media
			.filter(|m| !m.title.is_empty())
			.and_then(|m| m.volume)
			.map(|v| format!("{v}%"))
			.unwrap_or_default()
	};
	Some(match entry.kind {
		Kind::PlayPause => render::glyph_image(
			if media.is_some_and(|m| m.is_playing) {
				Glyph::Pause
			} else {
				Glyph::Play
			},
			render::FG,
			"",
		),
		Kind::Next => render::glyph_image(Glyph::Next, render::FG, ""),
		Kind::Previous => render::glyph_image(Glyph::Previous, render::FG, ""),
		Kind::VolumeUp => render::glyph_image(Glyph::VolumeUp, render::FG, &volume_label()),
		Kind::VolumeDown => render::glyph_image(Glyph::VolumeDown, render::FG, &volume_label()),
		Kind::Mute => {
			let volume = media.and_then(|m| m.volume);
			if volume == Some(0) {
				render::glyph_image(Glyph::Mute, render::RED, "Muted")
			} else {
				render::glyph_image(
					Glyph::Speaker,
					render::FG,
					&volume.map(|v| format!("{v}%")).unwrap_or_default(),
				)
			}
		}
		Kind::SeekBack | Kind::SeekForward => {
			let usable = media.is_none_or(|m| m.can_seek != Some(false));
			render::seek_image(
				entry.kind == Kind::SeekForward,
				&settings.seek_seconds.to_string(),
				if usable { render::FG } else { render::DIM },
			)
		}
		Kind::Battery => {
			let battery = source.battery.clone().unwrap_or_default();
			render::battery_image(battery.charge, battery.is_charging)
		}
		Kind::FindMyPhone => render::glyph_image(Glyph::FindMyPhone, render::FG, &device.name),
		Kind::Clipboard => render::glyph_image(Glyph::Clipboard, render::FG, "Send clipboard"),
		Kind::SwitchPlayer => {
			let players = media.map(|m| m.player_list.as_slice()).unwrap_or_default();
			let current = media.map(|m| m.player.as_str()).unwrap_or_default();
			let footer = if players.len() > 1 {
				let index = players.iter().position(|p| p == current).map_or(0, |i| i + 1);
				format!("{index} of {}", players.len())
			} else {
				String::new()
			};
			render::text_image("Player", if current.is_empty() { "None" } else { current }, &footer)
		}
		Kind::Notifications => {
			let (count, newest) = logic::visible_notifications(&source.notifications, settings.include_silent);
			render::notifications_image(count, &newest)
		}
		Kind::NowPlaying => {
			let media = media.cloned().unwrap_or_default();
			let progress = (!media.title.is_empty() && media.length > 0).then(|| {
				let position = source
					.fetched_at
					.map_or(media.position, |at| logic::estimated_position(&media, at, now));
				position as f64 / media.length as f64
			});
			let art = if settings.show_art {
				logic::visible_art(source.art.as_ref(), now)
			} else {
				None
			};
			render::now_playing_image(&NowPlaying {
				title: &media.title,
				artist: &media.artist,
				is_playing: media.is_playing,
				progress,
				art_data_uri: art,
				offset: entry.marquee_offset,
				show_text: settings.show_text || art.is_none(),
			})
		}
		Kind::RemoteCommand => {
			let command = source.commands.iter().find(|c| c.key == settings.command_key);
			let value = match command {
				Some(c) => c.name.as_str(),
				None if settings.command_key.is_empty() => "Choose…",
				None => "Missing",
			};
			render::text_image("Run", value, &device.name)
		}
		Kind::Lock => {
			if source.locked == Some(true) {
				render::glyph_image(Glyph::Locked, render::RED, "Locked")
			} else {
				render::glyph_image(Glyph::Unlocked, render::FG, &device.name)
			}
		}
		Kind::PcVolumeUp | Kind::PcVolumeDown | Kind::PcMute => {
			let Some(sink) = logic::selected_sink(&source.sinks, &settings.sink) else {
				return Some(render::text_image(&device.name, "No outputs", ""));
			};
			let label = if sink.muted {
				"Muted".to_string()
			} else {
				format!("{}%", logic::sink_percent(sink))
			};
			match entry.kind {
				Kind::PcMute if sink.muted => render::glyph_image(Glyph::Mute, render::RED, &label),
				Kind::PcMute => render::glyph_image(Glyph::Speaker, render::FG, &label),
				Kind::PcVolumeUp => render::glyph_image(Glyph::VolumeUp, render::FG, &label),
				_ => render::glyph_image(Glyph::VolumeDown, render::FG, &label),
			}
		}
		Kind::Call => render::call_image(logic::current_call(calls.get(&device.id), now).as_ref()),
		Kind::Signal => {
			let c = source.connectivity.clone().unwrap_or(Connectivity {
				strength: -1,
				network_type: String::new(),
			});
			render::signal_image(c.strength, &c.network_type)
		}
		Kind::Sms => {
			if entry.armed_until.is_some_and(|until| until > now) {
				render::text_image("Press again", "to send", "")
			} else {
				let value = [&settings.sms_label, &settings.sms_destination]
					.into_iter()
					.find(|s| !s.is_empty())
					.map_or("Set up…", |s| s.as_str());
				render::text_image("SMS", value, &settings.sms_message)
			}
		}
		Kind::Share => {
			let label = [&settings.share_label, &settings.share_target]
				.into_iter()
				.find(|s| !s.is_empty())
				.map_or("Set up…", |s| s.as_str());
			render::glyph_image(Glyph::Share, render::FG, label)
		}
		Kind::Ping => {
			let label = if settings.ping_message.is_empty() {
				device.name.as_str()
			} else {
				settings.ping_message.as_str()
			};
			render::glyph_image(Glyph::Ping, render::FG, label)
		}
	})
}

/// Renders the given keys (all when `only` is None) and sends changed images.
async fn render(only: Option<&[String]>) {
	let now = Instant::now();
	let updates: Vec<(String, String)> = {
		let mut guard = state();
		let State {
			entries,
			sources,
			calls,
		} = &mut *guard;
		entries
			.iter_mut()
			.filter(|(id, _)| only.is_none_or(|ids| ids.contains(id)))
			.filter_map(|(id, entry)| {
				let source = sources.get(&entry.settings.source_key(entry.kind))?;
				let image = render_entry(entry, source, calls, now)?;
				if entry.last_image.as_deref() == Some(image.as_str()) {
					return None;
				}
				entry.last_image = Some(image.clone());
				Some((id.clone(), image))
			})
			.collect()
	};
	for (id, image) in updates {
		if let Some(instance) = openaction::get_instance(id).await
			&& let Err(error) = instance.set_image(Some(image), None).await
		{
			log::warn!("setImage failed: {error}");
		}
	}
}

async fn render_all() {
	render(None).await;
}

// ------------------------------------------------------------------ polling

/// Resolves (and caches) the device for a source.
async fn ensure_device(key: &str) -> Result<Option<Device>> {
	let (device_id, plugin) = {
		let state = state();
		let Some(source) = state.sources.get(key) else {
			return Ok(None);
		};
		if let (Some(device), Some(at)) = (&source.device, source.resolved_at)
			&& at.elapsed() < DEVICE_CACHE
		{
			return Ok(Some(device.clone()));
		}
		(source.device_id.clone(), source.plugin)
	};
	let result = backend().resolve_device(&device_id, plugin).await;
	let mut state = state();
	if let Some(source) = state.sources.get_mut(key) {
		source.resolved_at = Some(Instant::now());
		source.device = result.as_ref().ok().cloned().flatten();
	}
	result
}

/// Album art failures (e.g. the file is still downloading) must not count as
/// the device disappearing, so they're handled apart from the other fetches.
async fn refresh_art(key: &str, device: &Device, media: &MediaState) {
	let art_key = logic::art_key(media);
	if media.title.is_empty() {
		return;
	}
	let previous = {
		let state = state();
		match state.sources.get(key) {
			Some(source) if source.art.as_ref().is_some_and(|a| a.key == art_key) => return,
			Some(source) => source.art.clone(),
			None => return,
		}
	};
	let path = match backend()
		.album_art_path(&device.name, &media.player, &media.title)
		.await
	{
		Ok(path) => path.filter(|p| logic::accept_art_path(p, previous.as_ref(), &media.album)),
		Err(error) => {
			log::debug!("album art lookup failed: {error:#}");
			None
		}
	};
	let fresh = match &path {
		Some(path) => match kdeconnect::read_album_art_data_uri(path).await {
			Ok(uri) => Some(Art {
				key: art_key,
				path: path.clone(),
				album: media.album.clone(),
				uri,
				pending_since: None,
			}),
			Err(error) => {
				log::debug!("album art not ready: {error:#}");
				None
			}
		},
		None => None,
	};
	let mut state = state();
	if let Some(source) = state.sources.get_mut(key) {
		// The art may arrive a moment after the track changes; only cache once we have it.
		source.art = match fresh {
			Some(art) => Some(art),
			None => logic::mark_art_pending(source.art.take(), Instant::now()),
		};
	}
}

/// Fetches only what the visible keys on this source need. Each feature fails
/// independently; a failure forces a fresh device lookup on the next poll,
/// which notices quickly if the device went away.
async fn refresh_source(key: String, needs: Needs) {
	let device = match ensure_device(&key).await {
		Ok(Some(device)) => device,
		Ok(None) => {
			if let Some(source) = state().sources.get_mut(&key) {
				source.media = None;
			}
			return;
		}
		Err(error) => {
			log::warn!("device lookup failed: {error:#}");
			return;
		}
	};
	let backend = backend();
	let has = |plugin: &str| device.plugins.iter().any(|p| p == plugin);
	let id = device.id.as_str();

	let media = async {
		if needs.media && has(MEDIA_PLUGIN) {
			Some(backend.media_state(id).await)
		} else {
			None
		}
	};
	let battery = async {
		if needs.battery && has("kdeconnect_battery") {
			Some(backend.battery(id).await)
		} else {
			None
		}
	};
	let lock = async {
		if needs.lock && has("kdeconnect_lockdevice") {
			Some(backend.locked(id).await)
		} else {
			None
		}
	};
	let sinks = async {
		if needs.sinks && has("kdeconnect_remotesystemvolume") {
			Some(backend.sinks(id).await)
		} else {
			None
		}
	};
	let connectivity = async {
		if needs.connectivity && has("kdeconnect_connectivity_report") {
			Some(backend.connectivity(id).await)
		} else {
			None
		}
	};
	let commands = async {
		if needs.commands && has("kdeconnect_remotecommands") {
			Some(backend.remote_commands(id).await)
		} else {
			None
		}
	};
	let notification_ids = async {
		if needs.notifications && has("kdeconnect_notifications") {
			Some(backend.notification_ids(id).await)
		} else {
			None
		}
	};
	let (media, battery, lock, sinks, connectivity, commands, notification_ids) =
		tokio::join!(media, battery, lock, sinks, connectivity, commands, notification_ids);

	// Notification details rarely change, so only fetch ones we haven't seen.
	let notifications = match notification_ids {
		Some(Ok(ids)) => {
			let known = state()
				.sources
				.get(&key)
				.map(|s| s.notifications.clone())
				.unwrap_or_default();
			let mut next = HashMap::new();
			for nid in ids {
				let info = match known.get(&nid) {
					Some(info) => info.clone(),
					None => backend.notification(id, &nid).await.unwrap_or_default(),
				};
				next.insert(nid, info);
			}
			Some(Ok(next))
		}
		Some(Err(error)) => Some(Err(error)),
		None => None,
	};

	let fetched_media = {
		let mut state = state();
		let Some(source) = state.sources.get_mut(&key) else {
			return;
		};
		let mut failed = false;
		let mut fetched_media = None;
		macro_rules! apply {
			($name:literal, $value:expr, |$ok:ident| $store:expr) => {
				match $value {
					Some(Ok($ok)) => $store,
					Some(Err(error)) => {
						log::warn!("{} failed: {error:#}", $name);
						failed = true;
					}
					None => {}
				}
			};
		}
		apply!("media", media, |m| {
			let mut m: MediaState = m;
			m.volume = logic::settled_volume(m.volume, source.volume_target, Instant::now());
			source.media = Some(m.clone());
			source.fetched_at = Some(Instant::now());
			fetched_media = Some(m);
		});
		apply!("battery", battery, |b| source.battery = Some(b));
		apply!("lock", lock, |l| source.locked = Some(l));
		apply!("sinks", sinks, |s| {
			let mut s: Vec<Sink> = s;
			// Keep showing our own value while the other computer catches up.
			if let Some((name, target, at)) = &source.sink_target
				&& at.elapsed() < logic::VOLUME_SETTLE
				&& let Some(sink) = s.iter_mut().find(|x| &x.name == name)
			{
				sink.volume = target.volume;
				sink.muted = target.muted;
			}
			source.sinks = s;
		});
		apply!("connectivity", connectivity, |c| source.connectivity = Some(c));
		apply!("commands", commands, |c| source.commands = c);
		apply!("notifications", notifications, |n| source.notifications = n);
		if failed {
			source.resolved_at = None;
		}
		fetched_media
	};
	if needs.art
		&& let Some(media) = fetched_media
	{
		refresh_art(&key, &device, &media).await;
	}
}

static POLLING: AtomicBool = AtomicBool::new(false);
static POLL_AGAIN: AtomicBool = AtomicBool::new(false);

/// Refreshes every source that a visible key uses, then re-renders. A poll
/// requested while one is running runs again right after it, since the
/// running one may have read the old state.
pub async fn poll() {
	if POLLING.swap(true, Ordering::SeqCst) {
		POLL_AGAIN.store(true, Ordering::SeqCst);
		return;
	}
	// Clears the flag however this poll ends, so a panic can't stop polling for good.
	struct Polling;
	impl Drop for Polling {
		fn drop(&mut self) {
			POLLING.store(false, Ordering::SeqCst);
		}
	}
	let _polling = Polling;
	loop {
		POLL_AGAIN.store(false, Ordering::SeqCst);
		let wanted: HashMap<String, Needs> = {
			let mut guard = state();
			let state = &mut *guard;
			let mut wanted: HashMap<String, Needs> = HashMap::new();
			let keys: Vec<(Kind, Settings)> = state.entries.values().map(|e| (e.kind, e.settings.clone())).collect();
			for (kind, settings) in keys {
				state.source_mut(kind, &settings);
				wanted
					.entry(settings.source_key(kind))
					.or_default()
					.add(kind, &settings);
			}
			wanted
		};
		futures_util::future::join_all(wanted.into_iter().map(|(key, needs)| refresh_source(key, needs))).await;
		render_all().await;
		if !POLL_AGAIN.load(Ordering::SeqCst) {
			break;
		}
	}
}

fn poll_soon(delay: Duration) {
	tokio::spawn(async move {
		tokio::time::sleep(delay).await;
		poll().await;
	});
}

async fn tick_marquee() {
	let ids: Vec<String> = {
		let mut guard = state();
		let State { entries, sources, .. } = &mut *guard;
		entries
			.iter_mut()
			.filter(|(_, e)| e.kind == Kind::NowPlaying && e.settings.show_text)
			.filter_map(|(id, entry)| {
				let media = sources
					.get(&entry.settings.source_key(entry.kind))
					.and_then(|s| s.media.as_ref());
				match media {
					Some(m) if render::now_playing_scrolls(&m.title, &m.artist) => {
						entry.marquee_offset += MARQUEE_STEP_PX;
						Some(id.clone())
					}
					_ => {
						entry.marquee_offset = 0.0;
						None
					}
				}
			})
			.collect()
	};
	if !ids.is_empty() {
		render(Some(&ids)).await;
	}
}

static SIGNAL_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// D-Bus signals arrive in bursts; coalesce them into one refresh.
fn on_signal(signal: Signal) {
	if let Signal::Call {
		device_id,
		event,
		number,
		name,
	} = signal
		&& !device_id.is_empty()
		&& !event.is_empty()
	{
		// telephony has no state to poll; its signal is the only source of calls.
		state().calls.insert(
			device_id,
			CallRecord {
				info: CallInfo { event, number, name },
				at: Instant::now(),
			},
		);
	}
	let generation = SIGNAL_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
	tokio::spawn(async move {
		tokio::time::sleep(SIGNAL_DEBOUNCE).await;
		if SIGNAL_GENERATION.load(Ordering::SeqCst) == generation {
			poll().await;
		}
	});
}

// ------------------------------------------------------------- key handling

/// The visible key's kind, settings and device, or an error to alert with.
async fn require_device(context: &str) -> Result<(Kind, Settings, String, Device)> {
	let (kind, settings) = {
		let state = state();
		let entry = state.entries.get(context).ok_or_else(|| anyhow!("key went away"))?;
		(entry.kind, entry.settings.clone())
	};
	let key = settings.source_key(kind);
	state().source_mut(kind, &settings);
	let device = ensure_device(&key)
		.await?
		.ok_or_else(|| anyhow!("no reachable KDE Connect device"))?;
	if !device.plugins.iter().any(|p| p == kind.plugin()) {
		return Err(anyhow!("{} doesn't support this", device.name));
	}
	Ok((kind, settings, key, device))
}

fn volume_lock(key: &str) -> Arc<tokio::sync::Mutex<()>> {
	state()
		.sources
		.get(key)
		.map(|s| s.volume_lock.clone())
		.unwrap_or_default()
}

/// One media-volume change: `compute(current, muted_from) -> (target, muted_from)`.
async fn change_volume(context: &str, compute: impl FnOnce(i32, Option<i32>) -> (i32, Option<i32>)) -> Result<()> {
	let (_, settings, key, device) = require_device(context).await?;
	let lock = volume_lock(&key);
	let _guard = lock.lock().await;
	let target = {
		let mut state = state();
		let source = state.sources.get_mut(&key).ok_or_else(|| anyhow!("no source"))?;
		let media = source.media.as_mut().ok_or_else(|| anyhow!("no media state yet"))?;
		let (target, muted_from) = compute(media.volume.unwrap_or(0), source.muted_from);
		media.volume = Some(target);
		source.muted_from = muted_from;
		source.volume_target = Some(VolumeTarget {
			volume: target,
			at: Instant::now(),
		});
		target
	};
	render_all().await;
	let backend = backend();
	// Volume applies to KDE Connect's selected player, so select the pinned one first.
	if !settings.player.is_empty() {
		backend.set_player(&device.id, &settings.player).await?;
	}
	backend.set_volume(&device.id, target).await
}

/// One change to another computer's output: `update` edits a copy of the sink.
async fn change_sink(context: &str, update: impl FnOnce(&mut Sink)) -> Result<()> {
	let (_, settings, key, device) = require_device(context).await?;
	let lock = volume_lock(&key);
	let _guard = lock.lock().await;
	let (before, after) = {
		let mut state = state();
		let source = state.sources.get_mut(&key).ok_or_else(|| anyhow!("no source"))?;
		let name = logic::selected_sink(&source.sinks, &settings.sink)
			.ok_or_else(|| anyhow!("no audio outputs"))?
			.name
			.clone();
		let sink = source
			.sinks
			.iter_mut()
			.find(|s| s.name == name)
			.expect("selected sink exists");
		let before = sink.clone();
		update(sink);
		let after = sink.clone();
		source.sink_target = Some((after.name.clone(), after.clone(), Instant::now()));
		(before, after)
	};
	render_all().await;
	let backend = backend();
	if (after.volume - before.volume).abs() > f64::EPSILON {
		backend.set_sink_volume(&device.id, &after.name, after.volume).await?;
	}
	if after.muted != before.muted {
		backend.set_sink_muted(&device.id, &after.name, after.muted).await?;
	}
	Ok(())
}

async fn step(context: &str) -> Result<()> {
	let (kind, step) = {
		let state = state();
		let entry = state.entries.get(context).ok_or_else(|| anyhow!("key went away"))?;
		(entry.kind, entry.settings.volume_step)
	};
	let direction = kind.hold_direction().unwrap_or(0);
	if kind.is_pc_volume() {
		change_sink(context, |sink| {
			sink.volume = logic::stepped_sink_volume(sink, direction, step)
		})
		.await
	} else {
		change_volume(context, |volume, muted_from| {
			(logic::stepped_volume(volume, direction, step), muted_from)
		})
		.await
	}
}

async fn alert(context: &str, error: &anyhow::Error) {
	log::warn!("{context}: {error:#}");
	if let Some(instance) = openaction::get_instance(context.to_string()).await {
		let _ = instance.show_alert().await;
	}
}

async fn ok(context: &str) {
	if let Some(instance) = openaction::get_instance(context.to_string()).await {
		let _ = instance.show_ok().await;
	}
}

fn stop_hold(context: &str) {
	if let Some(handle) = state().entries.get_mut(context).and_then(|e| e.hold.take()) {
		handle.abort();
	}
}

fn start_hold(context: String) {
	stop_hold(&context);
	// The first step runs in its own task so a quick tap's keyUp, which the SDK
	// delivers right behind keyDown, can't cancel it; only the repeats can.
	let first_context = context.clone();
	let first = tokio::spawn(async move {
		let result = step(&first_context).await;
		if let Err(error) = &result {
			alert(&first_context, error).await;
		}
		result.is_ok()
	});
	let task_context = context.clone();
	let handle = tokio::spawn(async move {
		let context = task_context;
		tokio::time::sleep(HOLD_DELAY).await;
		if !matches!(first.await, Ok(true)) {
			return;
		}
		loop {
			if let Err(error) = step(&context).await {
				alert(&context, &error).await;
				return;
			}
			tokio::time::sleep(HOLD_REPEAT).await;
		}
	});
	match state().entries.get_mut(&context) {
		Some(entry) => entry.hold = Some(handle),
		None => handle.abort(),
	}
}

/// Runs a key press. Returns whether to show the OK checkmark.
async fn press(context: &str) -> Result<bool> {
	let (kind, settings) = {
		let state = state();
		let entry = state.entries.get(context).ok_or_else(|| anyhow!("key went away"))?;
		(entry.kind, entry.settings.clone())
	};
	let backend = backend();
	match kind {
		Kind::Mute => {
			change_volume(context, logic::toggled_mute).await?;
			Ok(false)
		}
		Kind::PcMute => {
			change_sink(context, |sink| sink.muted = !sink.muted).await?;
			Ok(false)
		}
		Kind::SeekBack | Kind::SeekForward => {
			let (_, _, key, device) = require_device(context).await?;
			let (position, length) = {
				let state = state();
				let source = state.sources.get(&key).ok_or_else(|| anyhow!("no source"))?;
				let media = source
					.media
					.as_ref()
					.filter(|m| !m.title.is_empty())
					.ok_or_else(|| anyhow!("nothing playing"))?;
				if media.can_seek == Some(false) {
					return Err(anyhow!("player cannot seek"));
				}
				let position = source.fetched_at.map_or(media.position, |at| {
					logic::estimated_position(media, at, Instant::now())
				});
				(position, media.length)
			};
			if !settings.player.is_empty() {
				backend.set_player(&device.id, &settings.player).await?;
			}
			let target = logic::seek_target(position, length, kind == Kind::SeekForward, settings.seek_seconds);
			backend.set_position(&device.id, target).await?;
			if let Some(source) = state().sources.get_mut(&key)
				&& let Some(media) = source.media.as_mut()
			{
				media.position = target;
				source.fetched_at = Some(Instant::now());
			}
			render_all().await;
			Ok(false)
		}
		Kind::SwitchPlayer => {
			let (_, _, key, device) = require_device(context).await?;
			let next = {
				let state = state();
				let media = state
					.sources
					.get(&key)
					.and_then(|s| s.media.as_ref())
					.ok_or_else(|| anyhow!("no players"))?;
				if media.player_list.is_empty() {
					return Err(anyhow!("no players"));
				}
				let index = media
					.player_list
					.iter()
					.position(|p| p == &media.player)
					.map_or(0, |i| i + 1);
				media.player_list[index % media.player_list.len()].clone()
			};
			backend.set_player(&device.id, &next).await?;
			if let Some(source) = state().sources.get_mut(&key) {
				if let Some(media) = source.media.as_mut() {
					media.player = next;
				}
				source.art = None;
			}
			render_all().await;
			Ok(false)
		}
		Kind::FindMyPhone => {
			let (.., device) = require_device(context).await?;
			backend.ring(&device.id).await?;
			Ok(true)
		}
		Kind::Clipboard => {
			let (.., device) = require_device(context).await?;
			backend.send_clipboard(&device.id).await?;
			Ok(true)
		}
		Kind::Ping => {
			let (.., device) = require_device(context).await?;
			backend.send_ping(&device.id, &settings.ping_message).await?;
			Ok(true)
		}
		Kind::Lock => {
			let (_, _, key, device) = require_device(context).await?;
			let locked = !backend.locked(&device.id).await?;
			backend.set_locked(&device.id, locked).await?;
			if let Some(source) = state().sources.get_mut(&key) {
				source.locked = Some(locked);
			}
			render_all().await;
			Ok(false)
		}
		Kind::RemoteCommand => {
			let (.., device) = require_device(context).await?;
			if settings.command_key.is_empty() {
				return Err(anyhow!("no command chosen"));
			}
			backend
				.trigger_remote_command(&device.id, &settings.command_key)
				.await?;
			Ok(true)
		}
		// First press arms the key; only a second press within SMS_CONFIRM sends.
		Kind::Sms => {
			if settings.sms_destination.is_empty() || settings.sms_message.is_empty() {
				return Err(anyhow!("set a destination and message first"));
			}
			let armed_until = state().entries.get(context).and_then(|e| e.armed_until);
			match logic::sms_press(armed_until, Instant::now()) {
				SmsPress::Arm(until) => {
					if let Some(entry) = state().entries.get_mut(context) {
						entry.armed_until = Some(until);
					}
					render(Some(&[context.to_string()])).await;
					let redraw = context.to_string();
					tokio::spawn(async move {
						tokio::time::sleep(logic::SMS_CONFIRM + Duration::from_millis(50)).await;
						render(Some(&[redraw])).await;
					});
					Ok(false)
				}
				SmsPress::Send => {
					if let Some(entry) = state().entries.get_mut(context) {
						entry.armed_until = None;
					}
					render(Some(&[context.to_string()])).await;
					let (.., device) = require_device(context).await?;
					backend
						.send_sms(&device.id, &settings.sms_destination, &settings.sms_message)
						.await?;
					Ok(true)
				}
			}
		}
		Kind::Share => {
			let target = settings.share_target.trim();
			if target.is_empty() {
				return Err(anyhow!("nothing to share"));
			}
			let (.., device) = require_device(context).await?;
			if logic::is_url(target) {
				backend.share_url(&device.id, target).await?;
			} else {
				backend.share_text(&device.id, target).await?;
			}
			Ok(true)
		}
		// Pressing a call key dismisses a missed call / finished call display.
		Kind::Call => {
			let (.., device) = require_device(context).await?;
			state().calls.remove(&device.id);
			render_all().await;
			Ok(false)
		}
		Kind::Battery | Kind::Notifications | Kind::Signal => {
			poll().await;
			Ok(false)
		}
		Kind::PlayPause | Kind::Next | Kind::Previous | Kind::NowPlaying => {
			let action = match kind {
				Kind::PlayPause => "PlayPause",
				Kind::Next => "Next",
				Kind::Previous => "Previous",
				_ if settings.press_action == "none" => return Ok(false),
				_ => settings.press_action.as_str(),
			};
			let (_, _, key, device) = require_device(context).await?;
			let player = (!settings.player.is_empty()).then_some(settings.player.as_str());
			backend.send_action(&device.id, action, player).await?;
			// Optimistic feedback, then confirm with a real poll once the phone reacts.
			if action == "PlayPause"
				&& let Some(media) = state().sources.get_mut(&key).and_then(|s| s.media.as_mut())
			{
				media.is_playing = !media.is_playing;
			}
			render_all().await;
			Ok(false)
		}
		Kind::VolumeUp | Kind::VolumeDown | Kind::PcVolumeUp | Kind::PcVolumeDown => Ok(false),
	}
}

// --------------------------------------------------------- action callbacks

pub async fn on_will_appear(context: &str, kind: Kind, settings: &Json) {
	{
		let mut state = state();
		let settings = Settings::from_json(settings);
		state.source_mut(kind, &settings);
		state.entries.insert(
			context.to_string(),
			Entry {
				kind,
				settings,
				last_image: None,
				marquee_offset: 0.0,
				armed_until: None,
				hold: None,
			},
		);
	}
	render(Some(&[context.to_string()])).await;
	tokio::spawn(poll());
}

pub async fn on_will_disappear(context: &str) {
	stop_hold(context);
	state().entries.remove(context);
}

pub async fn on_did_receive_settings(context: &str, settings: &Json) {
	{
		let mut state = state();
		let settings = Settings::from_json(settings);
		let Some(kind) = state.entries.get(context).map(|e| e.kind) else {
			return;
		};
		state.source_mut(kind, &settings);
		if let Some(entry) = state.entries.get_mut(context) {
			entry.settings = settings;
			entry.last_image = None;
		}
	}
	tokio::spawn(poll());
}

pub async fn on_key_down(context: &str) {
	let kind = state().entries.get(context).map(|e| e.kind);
	let Some(kind) = kind else {
		return;
	};
	if kind.hold_direction().is_some() {
		start_hold(context.to_string());
		return;
	}
	// The SDK handles events one at a time, so run presses off its loop.
	let context = context.to_string();
	tokio::spawn(async move {
		match press(&context).await {
			Ok(confirm) => {
				if confirm {
					ok(&context).await;
				}
				poll_soon(AFTER_PRESS);
			}
			Err(error) => alert(&context, &error).await,
		}
	});
}

pub async fn on_key_up(context: &str) {
	stop_hold(context);
}

/// Device list, players, commands and outputs for the settings panel.
pub async fn send_inspector_data(context: &str) {
	let backend = backend();
	let devices = backend.list_devices().await.unwrap_or_default();
	let Some((kind, settings)) = state().entries.get(context).map(|e| (e.kind, e.settings.clone())) else {
		return;
	};
	let key = settings.source_key(kind);
	state().source_mut(kind, &settings);
	// Look the device up directly: an in-flight poll may predate this key.
	let device = ensure_device(&key).await.ok().flatten();
	let (commands, sinks) = match &device {
		Some(d) if kind == Kind::RemoteCommand => {
			(backend.remote_commands(&d.id).await.unwrap_or_default(), Vec::new())
		}
		Some(d) if kind.is_pc_volume() => (Vec::new(), backend.sinks(&d.id).await.unwrap_or_default()),
		_ => (Vec::new(), Vec::new()),
	};
	let players = state()
		.sources
		.get(&key)
		.and_then(|s| s.media.as_ref())
		.map(|m| m.player_list.clone())
		.unwrap_or_default();
	let supported = device
		.as_ref()
		.is_some_and(|d| d.plugins.iter().any(|p| p == kind.plugin()));
	let payload = json!({
		"type": "inspectorData",
		"devices": devices.iter().map(|d| json!({"id": d.id, "name": d.name})).collect::<Vec<_>>(),
		"supported": supported,
		"deviceName": device.as_ref().map(|d| d.name.clone()).unwrap_or_default(),
		"players": players,
		"commands": commands.iter().map(|c| json!({"value": c.key, "label": c.name})).collect::<Vec<_>>(),
		"sinks": sinks.iter().map(|s| json!({"value": s.name, "label": if s.description.is_empty() { &s.name } else { &s.description }})).collect::<Vec<_>>(),
	});
	if let Some(instance) = openaction::get_instance(context.to_string()).await
		&& let Err(error) = instance.send_to_property_inspector(payload).await
	{
		log::warn!("sendToPropertyInspector failed: {error}");
	}
}

pub async fn on_send_to_plugin(context: &str, payload: &Json) {
	match payload.get("type").and_then(Json::as_str) {
		Some("refresh") => {
			state().sources.clear();
			send_inspector_data(context).await;
			tokio::spawn(poll());
		}
		Some("inspect") => send_inspector_data(context).await,
		_ => {}
	}
}

/// Connects to D-Bus and starts the background loops. Call before `openaction::run`.
pub async fn start() -> Result<()> {
	let backend = Arc::new(Backend::connect().await?);
	BACKEND
		.set(backend.clone())
		.map_err(|_| anyhow!("plugin started twice"))?;
	// Signals give near-instant updates (and are the only source of call
	// events); the poll keeps progress moving and covers anything they miss.
	backend.watch(on_signal);
	tokio::spawn(async {
		let mut interval = tokio::time::interval(POLL_INTERVAL);
		loop {
			interval.tick().await;
			poll().await;
		}
	});
	tokio::spawn(async {
		let mut interval = tokio::time::interval(MARQUEE_INTERVAL);
		loop {
			interval.tick().await;
			tick_marquee().await;
		}
	});
	Ok(())
}
