//! A fake KDE Connect daemon with one paired "desktop" computer, for testing
//! the computer-only keys (remote commands, lock, PC volume) without a second
//! machine. Run it on a private bus, never the real session bus:
//!
//!     dbus-run-session -- sh -c 'cargo run --example fake_kdeconnect & ...'
//!
//! Every call it receives is printed as `FAKE ...`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::json;
use zbus::{connection, interface};

const DEVICE: &str = "fakepc";

struct Daemon;

#[interface(name = "org.kde.kdeconnect.daemon")]
impl Daemon {
	#[zbus(name = "deviceNames")]
	fn device_names(&self, _only_reachable: bool, _only_paired: bool) -> HashMap<String, String> {
		HashMap::from([(DEVICE.to_string(), "desktop".to_string())])
	}
}

struct Device;

#[interface(name = "org.kde.kdeconnect.device")]
impl Device {
	#[zbus(name = "loadedPlugins")]
	fn loaded_plugins(&self) -> Vec<String> {
		[
			"kdeconnect_remotecommands",
			"kdeconnect_lockdevice",
			"kdeconnect_remotesystemvolume",
			"kdeconnect_ping",
		]
		.map(String::from)
		.to_vec()
	}
}

struct RemoteCommands;

#[interface(name = "org.kde.kdeconnect.device.remotecommands")]
impl RemoteCommands {
	#[zbus(property, name = "commands")]
	fn commands(&self) -> Vec<u8> {
		json!({
			"abc": {"name": "Suspend", "command": "systemctl suspend"},
			"def": {"name": "Kill Steam", "command": "pkill steam"}
		})
		.to_string()
		.into_bytes()
	}

	#[zbus(name = "triggerCommand")]
	fn trigger_command(&self, key: String) {
		println!("FAKE triggerCommand {key}");
	}
}

struct LockDevice(Arc<Mutex<bool>>);

#[interface(name = "org.kde.kdeconnect.device.lockdevice")]
impl LockDevice {
	#[zbus(property, name = "isLocked")]
	fn is_locked(&self) -> bool {
		*self.0.lock().unwrap()
	}

	#[zbus(name = "setLocked")]
	fn set_locked(&self, locked: bool) {
		println!("FAKE setLocked {locked}");
		*self.0.lock().unwrap() = locked;
	}
}

#[derive(Clone)]
struct SinkState {
	name: &'static str,
	description: &'static str,
	volume: i32,
	muted: bool,
	enabled: bool,
}

struct SystemVolume(Arc<Mutex<Vec<SinkState>>>);

#[interface(name = "org.kde.kdeconnect.device.remotesystemvolume")]
impl SystemVolume {
	#[zbus(property, name = "sinks")]
	fn sinks(&self) -> Vec<u8> {
		let sinks: Vec<_> = self
			.0
			.lock()
			.unwrap()
			.iter()
			.map(|s| json!({"name": s.name, "description": s.description, "volume": s.volume, "maxVolume": 65536, "muted": s.muted, "enabled": s.enabled}))
			.collect();
		serde_json::to_vec(&sinks).unwrap()
	}

	#[zbus(name = "sendVolume")]
	fn send_volume(&self, name: String, volume: i32) {
		println!("FAKE sendVolume {name} {volume}");
		if let Some(s) = self.0.lock().unwrap().iter_mut().find(|s| s.name == name) {
			s.volume = volume;
		}
	}

	#[zbus(name = "sendMuted")]
	fn send_muted(&self, name: String, muted: bool) {
		println!("FAKE sendMuted {name} {muted}");
		if let Some(s) = self.0.lock().unwrap().iter_mut().find(|s| s.name == name) {
			s.muted = muted;
		}
	}
}

struct Ping;

#[interface(name = "org.kde.kdeconnect.device.ping")]
impl Ping {
	#[zbus(name = "sendPing")]
	fn send_ping(&self, message: String) {
		println!("FAKE sendPing {message}");
	}
}

#[tokio::main]
async fn main() -> zbus::Result<()> {
	let base = format!("/modules/kdeconnect/devices/{DEVICE}");
	let sinks = vec![
		SinkState {
			name: "alsa_output.hdmi",
			description: "HDMI",
			volume: 32768,
			muted: false,
			enabled: false,
		},
		SinkState {
			name: "alsa_output.speakers",
			description: "Speakers",
			volume: 26214,
			muted: false,
			enabled: true,
		},
	];
	let _conn = connection::Builder::session()
		.name("org.kde.kdeconnect")
		.serve_at("/modules/kdeconnect", Daemon)
		.serve_at(base.as_str(), Device)
		.serve_at(format!("{base}/remotecommands").as_str(), RemoteCommands)
		.serve_at(
			format!("{base}/lockdevice").as_str(),
			LockDevice(Arc::new(Mutex::new(false))),
		)
		.serve_at(
			format!("{base}/remotesystemvolume").as_str(),
			SystemVolume(Arc::new(Mutex::new(sinks))),
		)
		.serve_at(format!("{base}/ping").as_str(), Ping)
		.build()
		.await?;
	println!("FAKE ready");
	std::future::pending::<()>().await;
	Ok(())
}
