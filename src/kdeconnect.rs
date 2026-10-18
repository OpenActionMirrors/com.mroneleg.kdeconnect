//! Talks to the KDE Connect daemon (and the MPRIS bridges it exports) over the
//! session D-Bus.
//!
//! Writes go through [`Backend::write`] so that `KDC_DRY_RUN=1` can log them
//! instead of performing them: the test harness uses that to exercise every
//! key against a real phone without ringing it, changing its volume, etc.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::Value as Json;
use zbus::{Connection, DynamicType, MatchRule, MessageStream, OwnedVariant, message::Type as MessageType};

const SERVICE: &str = "org.kde.kdeconnect";
const DAEMON_PATH: &str = "/modules/kdeconnect";
const DAEMON_IFACE: &str = "org.kde.kdeconnect.daemon";
const DEVICE_IFACE: &str = "org.kde.kdeconnect.device";
const PROPERTIES_IFACE: &str = "org.freedesktop.DBus.Properties";
const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.kdeconnect.";

/// Actions understood by the Android side of the mprisremote plugin.
pub const MEDIA_ACTIONS: [&str; 6] = ["PlayPause", "Play", "Pause", "Next", "Previous", "Stop"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceName {
	pub id: String,
	pub name: String,
}

/// A reachable device and the KDE Connect plugins enabled for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
	pub id: String,
	pub name: String,
	pub plugins: Vec<String>,
}

/// State of the remote media player (mprisremote).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaState {
	pub player: String,
	pub player_list: Vec<String>,
	pub title: String,
	pub artist: String,
	pub album: String,
	pub is_playing: bool,
	/// Track length in ms.
	pub length: i64,
	/// Position in ms.
	pub position: i64,
	pub volume: Option<i32>,
	pub can_seek: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Battery {
	pub charge: Option<i32>,
	pub is_charging: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationInfo {
	pub app_name: String,
	pub silent: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteCommand {
	pub key: String,
	pub name: String,
	pub command: String,
}

/// One of another computer's audio outputs.
#[derive(Clone, Debug, PartialEq)]
pub struct Sink {
	pub name: String,
	pub description: String,
	pub volume: f64,
	pub max_volume: f64,
	pub muted: bool,
	/// Set on the computer's default output.
	pub enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Connectivity {
	/// 0-4, or -1 when unknown.
	pub strength: i32,
	pub network_type: String,
}

/// A signal relevant to the plugin, from [`Backend::watch`].
#[derive(Clone, Debug)]
pub enum Signal {
	/// telephony's callReceived(event, number, contactName) for a device.
	Call {
		device_id: String,
		event: String,
		number: String,
		name: String,
	},
	/// Anything else under /modules/kdeconnect/devices or an MPRIS bridge.
	Changed,
}

type VariantMap = HashMap<String, OwnedVariant>;

fn take<T: TryFrom<OwnedVariant>>(map: &mut VariantMap, key: &str) -> Option<T> {
	map.remove(key).and_then(|value| T::try_from(value).ok())
}

fn device_path(device_id: &str) -> String {
	format!("{DAEMON_PATH}/devices/{device_id}")
}

/// QByteArray properties arrive as `ay`: bytes holding JSON.
pub fn decode_json_bytes(bytes: &[u8]) -> Option<Json> {
	if bytes.is_empty() {
		return None;
	}
	serde_json::from_slice(bytes).ok()
}

/// remotecommands' `commands`: {"<key>": {"name": ..., "command": ...}}.
pub fn parse_commands(json: Option<Json>) -> Vec<RemoteCommand> {
	let Some(Json::Object(map)) = json else {
		return Vec::new();
	};
	map.into_iter()
		.map(|(key, value)| RemoteCommand {
			name: value
				.get("name")
				.and_then(Json::as_str)
				.filter(|n| !n.is_empty())
				.unwrap_or(&key)
				.to_string(),
			command: value
				.get("command")
				.and_then(Json::as_str)
				.unwrap_or_default()
				.to_string(),
			key,
		})
		.collect()
}

/// remotesystemvolume's `sinks`: [{name, description, volume, maxVolume, muted, enabled}].
pub fn parse_sinks(json: Option<Json>) -> Vec<Sink> {
	let Some(Json::Array(items)) = json else {
		return Vec::new();
	};
	items
		.iter()
		.filter_map(|item| {
			let name = item.get("name")?.as_str()?.to_string();
			Some(Sink {
				description: item
					.get("description")
					.and_then(Json::as_str)
					.unwrap_or_default()
					.to_string(),
				volume: item.get("volume").and_then(Json::as_f64).unwrap_or(0.0),
				max_volume: item.get("maxVolume").and_then(Json::as_f64).unwrap_or(0.0),
				muted: item.get("muted").and_then(Json::as_bool).unwrap_or(false),
				enabled: item.get("enabled").and_then(Json::as_bool).unwrap_or(false),
				name,
			})
		})
		.collect()
}

fn media_from_map(mut map: VariantMap) -> MediaState {
	MediaState {
		player: take(&mut map, "player").unwrap_or_default(),
		player_list: take(&mut map, "playerList").unwrap_or_default(),
		title: take(&mut map, "title").unwrap_or_default(),
		artist: take(&mut map, "artist").unwrap_or_default(),
		album: take(&mut map, "album").unwrap_or_default(),
		is_playing: take(&mut map, "isPlaying").unwrap_or(false),
		length: take::<i32>(&mut map, "length").map(i64::from).unwrap_or(0),
		position: take::<i32>(&mut map, "position").map(i64::from).unwrap_or(0),
		volume: take(&mut map, "volume"),
		can_seek: take(&mut map, "canSeek"),
	}
}

#[derive(Clone)]
pub struct Backend {
	conn: Connection,
	dry_run: bool,
}

impl Backend {
	pub async fn connect() -> Result<Self> {
		let conn = Connection::session().await.context("connecting to the session bus")?;
		let dry_run = std::env::var("KDC_DRY_RUN").is_ok_and(|v| !v.is_empty() && v != "0");
		if dry_run {
			log::warn!("KDC_DRY_RUN set: writes are logged, not performed");
		}
		Ok(Self { conn, dry_run })
	}

	async fn call<B, R>(&self, dest: &str, path: &str, iface: &str, method: &str, body: &B) -> Result<R>
	where
		B: Serialize + DynamicType,
		R: for<'de> serde::Deserialize<'de> + zbus::Type,
	{
		let reply = self
			.conn
			.call_method(Some(dest), path, Some(iface), method, body)
			.await
			.with_context(|| format!("{iface}.{method} on {path}"))?;
		Ok(reply.body().deserialize::<R>()?)
	}

	async fn call_void<B: Serialize + DynamicType>(
		&self,
		dest: &str,
		path: &str,
		iface: &str,
		method: &str,
		body: &B,
	) -> Result<()> {
		self.conn
			.call_method(Some(dest), path, Some(iface), method, body)
			.await
			.with_context(|| format!("{iface}.{method} on {path}"))?;
		Ok(())
	}

	async fn get_property<T: TryFrom<OwnedVariant>>(
		&self,
		dest: &str,
		path: &str,
		iface: &str,
		name: &str,
	) -> Result<T> {
		let value: OwnedVariant = self.call(dest, path, PROPERTIES_IFACE, "Get", &(iface, name)).await?;
		T::try_from(value).map_err(|_| anyhow!("unexpected type for {iface}.{name}"))
	}

	async fn get_all(&self, dest: &str, path: &str, iface: &str) -> Result<VariantMap> {
		self.call(dest, path, PROPERTIES_IFACE, "GetAll", &(iface,)).await
	}

	/// Performs a write, or just logs it under KDC_DRY_RUN.
	async fn write<F>(&self, description: String, write: F) -> Result<()>
	where
		F: Future<Output = Result<()>>,
	{
		if self.dry_run {
			log::info!("DRY RUN: {description}");
			return Ok(());
		}
		write.await
	}

	async fn set_property<V>(&self, path: &str, iface: &str, name: &str, value: V) -> Result<()>
	where
		V: Into<zbus::Variant<'static>> + std::fmt::Debug + Clone,
	{
		let description = format!("set {iface}.{name} = {value:?} on {path}");
		self.write(description, async {
			let variant: zbus::Variant<'static> = value.into();
			self.call_void(SERVICE, path, PROPERTIES_IFACE, "Set", &(iface, name, variant))
				.await
		})
		.await
	}

	async fn plugin_call<B>(&self, device_id: &str, plugin: &str, method: &str, body: B) -> Result<()>
	where
		B: Serialize + DynamicType + std::fmt::Debug,
	{
		let path = format!("{}/{plugin}", device_path(device_id));
		let iface = format!("{DEVICE_IFACE}.{plugin}");
		let description = format!("{iface}.{method}{body:?} on {path}");
		self.write(description, self.call_void(SERVICE, &path, &iface, method, &body))
			.await
	}

	async fn plugin_property<T: TryFrom<OwnedVariant>>(&self, device_id: &str, plugin: &str, name: &str) -> Result<T> {
		let path = format!("{}/{plugin}", device_path(device_id));
		self.get_property(SERVICE, &path, &format!("{DEVICE_IFACE}.{plugin}"), name)
			.await
	}

	/// Paired and reachable devices.
	pub async fn list_devices(&self) -> Result<Vec<DeviceName>> {
		let names: HashMap<String, String> = self
			.call(SERVICE, DAEMON_PATH, DAEMON_IFACE, "deviceNames", &(true, true))
			.await?;
		let mut devices: Vec<DeviceName> = names.into_iter().map(|(id, name)| DeviceName { id, name }).collect();
		devices.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
		Ok(devices)
	}

	/// KDE Connect plugin ids (e.g. "kdeconnect_battery") enabled for a device.
	pub async fn loaded_plugins(&self, device_id: &str) -> Result<Vec<String>> {
		self.call(SERVICE, &device_path(device_id), DEVICE_IFACE, "loadedPlugins", &())
			.await
	}

	/// The explicit device if given, otherwise the first reachable device with
	/// `plugin` enabled.
	pub async fn resolve_device(&self, device_id: &str, plugin: Option<&str>) -> Result<Option<Device>> {
		for device in self.list_devices().await? {
			if !device_id.is_empty() && device.id != device_id {
				continue;
			}
			let plugins = self.loaded_plugins(&device.id).await.unwrap_or_default();
			if !device_id.is_empty() || plugin.is_none_or(|p| plugins.iter().any(|loaded| loaded == p)) {
				return Ok(Some(Device {
					id: device.id,
					name: device.name,
					plugins,
				}));
			}
		}
		Ok(None)
	}

	fn mpris_path(device_id: &str) -> String {
		format!("{}/mprisremote", device_path(device_id))
	}

	pub async fn media_state(&self, device_id: &str) -> Result<MediaState> {
		let map = self
			.get_all(
				SERVICE,
				&Self::mpris_path(device_id),
				&format!("{DEVICE_IFACE}.mprisremote"),
			)
			.await?;
		Ok(media_from_map(map))
	}

	pub async fn set_player(&self, device_id: &str, player: &str) -> Result<()> {
		self.set_property(
			&Self::mpris_path(device_id),
			&format!("{DEVICE_IFACE}.mprisremote"),
			"player",
			player.to_string(),
		)
		.await
	}

	pub async fn send_action(&self, device_id: &str, action: &str, player: Option<&str>) -> Result<()> {
		if !MEDIA_ACTIONS.contains(&action) {
			return Err(anyhow!("unknown media action: {action}"));
		}
		if let Some(player) = player.filter(|p| !p.is_empty()) {
			self.set_player(device_id, player).await?;
		}
		self.plugin_call(device_id, "mprisremote", "sendAction", (action.to_string(),))
			.await
	}

	pub async fn set_volume(&self, device_id: &str, volume: i32) -> Result<()> {
		self.set_property(
			&Self::mpris_path(device_id),
			&format!("{DEVICE_IFACE}.mprisremote"),
			"volume",
			volume.clamp(0, 100),
		)
		.await
	}

	/// Absolute position in ms (same unit as `position`/`length`).
	pub async fn set_position(&self, device_id: &str, position_ms: i64) -> Result<()> {
		let position = i32::try_from(position_ms.max(0)).unwrap_or(i32::MAX);
		self.set_property(
			&Self::mpris_path(device_id),
			&format!("{DEVICE_IFACE}.mprisremote"),
			"position",
			position,
		)
		.await
	}

	pub async fn battery(&self, device_id: &str) -> Result<Battery> {
		let path = format!("{}/battery", device_path(device_id));
		let mut map = self.get_all(SERVICE, &path, &format!("{DEVICE_IFACE}.battery")).await?;
		Ok(Battery {
			charge: take(&mut map, "charge"),
			is_charging: take(&mut map, "isCharging").unwrap_or(false),
		})
	}

	/// Makes the device ring until it's acknowledged on the device.
	pub async fn ring(&self, device_id: &str) -> Result<()> {
		self.plugin_call(device_id, "findmyphone", "ring", ()).await
	}

	/// Pushes this computer's clipboard to the device.
	pub async fn send_clipboard(&self, device_id: &str) -> Result<()> {
		self.plugin_call(device_id, "clipboard", "sendClipboard", ()).await
	}

	pub async fn notification_ids(&self, device_id: &str) -> Result<Vec<String>> {
		let path = format!("{}/notifications", device_path(device_id));
		self.call(
			SERVICE,
			&path,
			&format!("{DEVICE_IFACE}.notifications"),
			"activeNotifications",
			&(),
		)
		.await
	}

	pub async fn notification(&self, device_id: &str, id: &str) -> Result<NotificationInfo> {
		let path = format!("{}/notifications/{id}", device_path(device_id));
		let mut map = self
			.get_all(SERVICE, &path, &format!("{DEVICE_IFACE}.notifications.notification"))
			.await?;
		Ok(NotificationInfo {
			app_name: take(&mut map, "appName").unwrap_or_default(),
			silent: take(&mut map, "silent").unwrap_or(false),
		})
	}

	/// Commands another computer offers.
	pub async fn remote_commands(&self, device_id: &str) -> Result<Vec<RemoteCommand>> {
		let bytes: Vec<u8> = self.plugin_property(device_id, "remotecommands", "commands").await?;
		Ok(parse_commands(decode_json_bytes(&bytes)))
	}

	pub async fn trigger_remote_command(&self, device_id: &str, key: &str) -> Result<()> {
		self.plugin_call(device_id, "remotecommands", "triggerCommand", (key.to_string(),))
			.await
	}

	pub async fn locked(&self, device_id: &str) -> Result<bool> {
		self.plugin_property(device_id, "lockdevice", "isLocked").await
	}

	pub async fn set_locked(&self, device_id: &str, locked: bool) -> Result<()> {
		self.plugin_call(device_id, "lockdevice", "setLocked", (locked,)).await
	}

	/// Another computer's audio outputs.
	pub async fn sinks(&self, device_id: &str) -> Result<Vec<Sink>> {
		let bytes: Vec<u8> = self.plugin_property(device_id, "remotesystemvolume", "sinks").await?;
		Ok(parse_sinks(decode_json_bytes(&bytes)))
	}

	pub async fn set_sink_volume(&self, device_id: &str, sink: &str, volume: f64) -> Result<()> {
		self.plugin_call(
			device_id,
			"remotesystemvolume",
			"sendVolume",
			(sink.to_string(), volume.round() as i32),
		)
		.await
	}

	pub async fn set_sink_muted(&self, device_id: &str, sink: &str, muted: bool) -> Result<()> {
		self.plugin_call(device_id, "remotesystemvolume", "sendMuted", (sink.to_string(), muted))
			.await
	}

	pub async fn connectivity(&self, device_id: &str) -> Result<Connectivity> {
		let strength: i32 = self
			.plugin_property(device_id, "connectivity_report", "cellularNetworkStrength")
			.await?;
		let network_type: String = self
			.plugin_property(device_id, "connectivity_report", "cellularNetworkType")
			.await?;
		Ok(Connectivity { strength, network_type })
	}

	/// sendSms takes D-Bus structs for addresses; kdeconnect-cli does that
	/// encoding, and is what KDE Connect itself documents for scripting SMS.
	pub async fn send_sms(&self, device_id: &str, destination: &str, message: &str) -> Result<()> {
		let description = format!("kdeconnect-cli -d {device_id} --send-sms {message:?} --destination {destination:?}");
		self.write(description, async {
			let status = tokio::process::Command::new("kdeconnect-cli")
				.args(["-d", device_id, "--send-sms", message, "--destination", destination])
				.status()
				.await
				.context("running kdeconnect-cli")?;
			if status.success() {
				Ok(())
			} else {
				Err(anyhow!("kdeconnect-cli exited with {status}"))
			}
		})
		.await
	}

	pub async fn share_url(&self, device_id: &str, url: &str) -> Result<()> {
		self.plugin_call(device_id, "share", "shareUrl", (url.to_string(),))
			.await
	}

	pub async fn share_text(&self, device_id: &str, text: &str) -> Result<()> {
		self.plugin_call(device_id, "share", "shareText", (text.to_string(),))
			.await
	}

	pub async fn send_ping(&self, device_id: &str, message: &str) -> Result<()> {
		if message.is_empty() {
			self.plugin_call(device_id, "ping", "sendPing", ()).await
		} else {
			self.plugin_call(device_id, "ping", "sendPing", (message.to_string(),))
				.await
		}
	}

	/// KDE Connect re-exports each remote player as a local MPRIS player whose
	/// Identity is "<player> - <device name>". That bridge is the only place the
	/// downloaded album art is exposed. Returns None while the bridge still
	/// shows another track (it briefly lags behind a track change).
	pub async fn album_art_path(&self, device_name: &str, player: &str, title: &str) -> Result<Option<PathBuf>> {
		let names: Vec<String> = self
			.call(
				"org.freedesktop.DBus",
				"/org/freedesktop/DBus",
				"org.freedesktop.DBus",
				"ListNames",
				&(),
			)
			.await?;
		let identity_wanted = format!("{player} - {device_name}");
		for name in names.iter().filter(|n| n.starts_with(MPRIS_PREFIX)) {
			let Ok(identity) = self
				.get_property::<String>(name, "/org/mpris/MediaPlayer2", "org.mpris.MediaPlayer2", "Identity")
				.await
			else {
				continue;
			};
			if identity != identity_wanted {
				continue;
			}
			let Ok(mut metadata) = self
				.get_property::<VariantMap>(
					name,
					"/org/mpris/MediaPlayer2",
					"org.mpris.MediaPlayer2.Player",
					"Metadata",
				)
				.await
			else {
				continue;
			};
			let bridge_title: Option<String> = take(&mut metadata, "xesam:title");
			if !title.is_empty() && bridge_title.as_deref() != Some(title) {
				return Ok(None);
			}
			let art_url: Option<String> = take(&mut metadata, "mpris:artUrl");
			if let Some(path) = art_url.as_deref().and_then(|u| u.strip_prefix("file://")) {
				return Ok(Some(PathBuf::from(percent_decode(path))));
			}
		}
		Ok(None)
	}

	/// Watches for KDE Connect device signals and MPRIS bridge changes. Calls
	/// `on_signal` for each; reconnects the stream if it ends.
	pub fn watch(self: Arc<Self>, on_signal: impl Fn(Signal) + Send + Sync + 'static) {
		tokio::spawn(async move {
			loop {
				if let Err(error) = self.watch_once(&on_signal).await {
					log::warn!("signal watch failed: {error:#}");
				}
				tokio::time::sleep(std::time::Duration::from_secs(5)).await;
			}
		});
	}

	async fn watch_once(&self, on_signal: &(impl Fn(Signal) + Send + Sync)) -> Result<()> {
		let devices_namespace = format!("{DAEMON_PATH}/devices");
		let devices = MatchRule::builder()
			.msg_type(MessageType::Signal)
			.path_namespace(devices_namespace.as_str())
			.build()?;
		let mpris = MatchRule::builder()
			.msg_type(MessageType::Signal)
			.interface(PROPERTIES_IFACE)
			.member("PropertiesChanged")
			.path("/org/mpris/MediaPlayer2")
			.build()?;
		let mut device_stream = MessageStream::for_match_rule(devices, &self.conn, None).await?;
		let mut mpris_stream = MessageStream::for_match_rule(mpris, &self.conn, None).await?;
		loop {
			let message = tokio::select! {
				Some(message) = device_stream.next() => message?,
				Some(message) = mpris_stream.next() => message?,
				else => return Ok(()),
			};
			let header = message.header();
			if header.member().is_some_and(|m| m.as_str() == "callReceived") {
				let device_id = header
					.path()
					.map(|p| p.as_str().split('/').nth(4).unwrap_or_default().to_string())
					.unwrap_or_default();
				if let Ok((event, number, name)) = message.body().deserialize::<(String, String, String)>() {
					on_signal(Signal::Call {
						device_id,
						event,
						number,
						name,
					});
					continue;
				}
			}
			on_signal(Signal::Changed);
		}
	}
}

/// Minimal %XX decoding for file:// URLs (album art lives in the user's cache).
fn percent_decode(path: &str) -> String {
	let bytes = path.as_bytes();
	let mut out = Vec::with_capacity(bytes.len());
	let mut i = 0;
	while i < bytes.len() {
		let escaped = (bytes[i] == b'%' && i + 2 < bytes.len())
			.then(|| std::str::from_utf8(&bytes[i + 1..i + 3]).ok())
			.flatten()
			.and_then(|hex| u8::from_str_radix(hex, 16).ok());
		if let Some(byte) = escaped {
			out.push(byte);
			i += 3;
			continue;
		}
		out.push(bytes[i]);
		i += 1;
	}
	String::from_utf8_lossy(&out).into_owned()
}

pub async fn read_album_art_data_uri(path: &std::path::Path) -> Result<String> {
	let bytes = tokio::fs::read(path)
		.await
		.with_context(|| format!("reading {}", path.display()))?;
	let mime = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")) {
		"image/png"
	} else {
		"image/jpeg"
	};
	Ok(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn decodes_commands_from_qbytearray_json() {
		let bytes = br#"{"abc":{"name":"Suspend","command":"systemctl suspend"},"def":{"name":"","command":"x"}}"#;
		let mut commands = parse_commands(decode_json_bytes(bytes));
		commands.sort_by(|a, b| a.key.cmp(&b.key));
		assert_eq!(
			commands[0],
			RemoteCommand {
				key: "abc".into(),
				name: "Suspend".into(),
				command: "systemctl suspend".into()
			}
		);
		// An empty name falls back to the key.
		assert_eq!(commands[1].name, "def");
	}

	#[test]
	fn bad_or_empty_bytes_mean_no_commands() {
		assert!(parse_commands(decode_json_bytes(b"")).is_empty());
		assert!(parse_commands(decode_json_bytes(b"not json")).is_empty());
		assert!(parse_commands(Some(json!([1, 2]))).is_empty());
	}

	#[test]
	fn decodes_sinks() {
		let sinks = parse_sinks(Some(json!([
			{"name": "hdmi", "description": "HDMI", "volume": 32768, "maxVolume": 65536, "muted": false, "enabled": false},
			{"name": "speakers", "description": "Speakers", "volume": 26214, "maxVolume": 65536, "muted": true, "enabled": true},
			{"description": "no name, skipped"}
		])));
		assert_eq!(sinks.len(), 2);
		assert_eq!(sinks[1].name, "speakers");
		assert!(sinks[1].muted && sinks[1].enabled);
		assert_eq!(sinks[0].volume, 32768.0);
	}

	#[test]
	fn percent_decodes_file_urls() {
		assert_eq!(percent_decode("/home/a%20b/art.jpg"), "/home/a b/art.jpg");
		assert_eq!(percent_decode("/plain/path%"), "/plain/path%");
	}
}
