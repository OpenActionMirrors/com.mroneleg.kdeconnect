//! One `openaction::Action` per manifest action. They all share the handlers
//! in [`crate::plugin`]; each type just fixes its UUID and [`Kind`].

use openaction::{Action, ActionUuid, Instance, OpenActionResult, async_trait};
use serde_json::Value as Json;

use crate::logic::Kind;
use crate::plugin;

macro_rules! actions {
	($($name:ident => $short:literal),* $(,)?) => {
		$(
			pub struct $name;

			#[async_trait]
			impl Action for $name {
				const UUID: ActionUuid = concat!("com.mroneleg.kdeconnect.", $short);
				// Parsed leniently by `Settings::from_json`, so one bad field
				// doesn't reset the rest.
				type Settings = Json;

				async fn will_appear(&self, instance: &Instance, settings: &Json) -> OpenActionResult<()> {
					plugin::on_will_appear(&instance.instance_id, kind($short), settings).await;
					Ok(())
				}

				async fn will_disappear(&self, instance: &Instance, _settings: &Json) -> OpenActionResult<()> {
					plugin::on_will_disappear(&instance.instance_id).await;
					Ok(())
				}

				async fn key_down(&self, instance: &Instance, _settings: &Json) -> OpenActionResult<()> {
					plugin::on_key_down(&instance.instance_id).await;
					Ok(())
				}

				async fn key_up(&self, instance: &Instance, _settings: &Json) -> OpenActionResult<()> {
					plugin::on_key_up(&instance.instance_id).await;
					Ok(())
				}

				async fn did_receive_settings(&self, instance: &Instance, settings: &Json) -> OpenActionResult<()> {
					plugin::on_did_receive_settings(&instance.instance_id, settings).await;
					Ok(())
				}

				async fn property_inspector_did_appear(&self, instance: &Instance, _settings: &Json) -> OpenActionResult<()> {
					let context = instance.instance_id.clone();
					tokio::spawn(async move { plugin::send_inspector_data(&context).await });
					Ok(())
				}

				async fn send_to_plugin(&self, instance: &Instance, _settings: &Json, payload: &Json) -> OpenActionResult<()> {
					let (context, payload) = (instance.instance_id.clone(), payload.clone());
					tokio::spawn(async move { plugin::on_send_to_plugin(&context, &payload).await });
					Ok(())
				}
			}
		)*

		/// Registers every action with the OpenAction runtime.
		pub async fn register_all() {
			$(openaction::register_action($name).await;)*
		}

		/// Manifest UUIDs, for tests.
		pub const UUIDS: &[&str] = &[$(<$name as Action>::UUID),*];
	};
}

fn kind(short: &str) -> Kind {
	Kind::from_short(short).expect("action short names match Kind")
}

actions! {
	PlayPause => "playpause",
	Next => "next",
	Previous => "previous",
	NowPlaying => "nowplaying",
	VolumeUp => "volumeup",
	VolumeDown => "volumedown",
	Mute => "mute",
	SeekBack => "seekback",
	SeekForward => "seekforward",
	SwitchPlayer => "switchplayer",
	Battery => "battery",
	FindMyPhone => "findmyphone",
	Notifications => "notifications",
	Clipboard => "clipboard",
	RemoteCommand => "remotecommand",
	Lock => "lock",
	PcVolumeUp => "pcvolumeup",
	PcVolumeDown => "pcvolumedown",
	PcMute => "pcmute",
	Call => "call",
	Signal => "signal",
	Sms => "sms",
	Share => "share",
	Ping => "ping",
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn every_action_maps_to_a_kind_and_matches_the_manifest() {
		let manifest: Json = serde_json::from_str(include_str!("../assets/manifest.json")).unwrap();
		let mut in_manifest: Vec<&str> = manifest["Actions"]
			.as_array()
			.unwrap()
			.iter()
			.map(|a| a["UUID"].as_str().unwrap())
			.collect();
		let mut registered: Vec<&str> = UUIDS.to_vec();
		in_manifest.sort();
		registered.sort();
		assert_eq!(in_manifest, registered);
		for uuid in UUIDS {
			assert!(plugin::kind_of(uuid).is_some(), "{uuid}");
		}
	}

	#[test]
	fn manifest_version_matches_cargo() {
		let manifest: Json = serde_json::from_str(include_str!("../assets/manifest.json")).unwrap();
		assert_eq!(manifest["Version"], env!("CARGO_PKG_VERSION"));
	}
}
