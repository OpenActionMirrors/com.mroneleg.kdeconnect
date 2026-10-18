//! Regenerates the static PNG icons in assets/icons from the same SVG
//! renderers the plugin uses at runtime, rasterised with resvg (the renderer
//! OpenDeck itself uses). Run from the repository root:
//!
//!     cargo run --features icons --bin build-icons

use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use kdeconnect_opendeck::render::{self, CallInfo, FG, Glyph, NowPlaying};
use resvg::{tiny_skia, usvg};

/// A laptop and a phone with a two-way arrow between them. `tile` draws it
/// white on a KDE-blue rounded tile (plugin icon); otherwise white on
/// transparent (category icon, shown small in the action list).
fn brand_icon(tile: bool) -> String {
	let background = if tile {
		r##"<defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#1d99f3"/><stop offset="1" stop-color="#1272b8"/></linearGradient></defs><rect width="144" height="144" rx="30" fill="url(#g)"/>"##
	} else {
		""
	};
	format!(
		r##"<svg xmlns="http://www.w3.org/2000/svg" width="144" height="144" viewBox="0 0 144 144">{background}<g fill="none" stroke="#fff" stroke-width="7" stroke-linecap="round" stroke-linejoin="round"><rect x="14" y="62" width="62" height="40" rx="5"/><rect x="98" y="54" width="32" height="58" rx="7"/><path d="M44 38 H112 M44 38 L56 28 M44 38 L56 48 M112 38 L100 28 M112 38 L100 48"/></g><path d="M6 108 H84 L78 116 H12 Z" fill="#fff"/><circle cx="114" cy="102" r="3.5" fill="#fff"/></svg>"##
	)
}

fn svg_of(data_uri: &str) -> String {
	let b64 = data_uri
		.strip_prefix("data:image/svg+xml;base64,")
		.expect("svg data uri");
	String::from_utf8(STANDARD.decode(b64).expect("base64")).expect("utf-8")
}

fn rasterise(svg: &str, size: u32, options: &usvg::Options) -> tiny_skia::Pixmap {
	let tree = usvg::Tree::from_str(svg, options).expect("valid svg");
	let mut pixmap = tiny_skia::Pixmap::new(size, size).expect("pixmap");
	let scale = size as f32 / tree.size().width();
	resvg::render(
		&tree,
		tiny_skia::Transform::from_scale(scale, scale),
		&mut pixmap.as_mut(),
	);
	pixmap
}

fn main() {
	let out_dir = Path::new("assets/icons");
	std::fs::create_dir_all(out_dir).expect("create assets/icons");
	let mut options = usvg::Options::default();
	options.fontdb_mut().load_system_fonts();

	let ringing = CallInfo {
		event: "ringing".into(),
		number: String::new(),
		name: "Caller".into(),
	};
	let action_icons: Vec<(&str, String)> = vec![
		("playpause", render::glyph_image(Glyph::Play, FG, "")),
		("next", render::glyph_image(Glyph::Next, FG, "")),
		("previous", render::glyph_image(Glyph::Previous, FG, "")),
		("volumeup", render::glyph_image(Glyph::VolumeUp, FG, "")),
		("volumedown", render::glyph_image(Glyph::VolumeDown, FG, "")),
		("mute", render::glyph_image(Glyph::Mute, FG, "")),
		("seekback", render::seek_image(false, "10", FG)),
		("seekforward", render::seek_image(true, "10", FG)),
		("switchplayer", render::text_image("Player", "Switch", "")),
		("battery", render::battery_image(Some(80), false)),
		("notifications", render::notifications_image(3, "Notifications")),
		("findmyphone", render::glyph_image(Glyph::FindMyPhone, FG, "")),
		("clipboard", render::glyph_image(Glyph::Clipboard, FG, "")),
		("remotecommand", render::text_image("Run", "Command", "")),
		("lock", render::glyph_image(Glyph::Locked, FG, "")),
		("pcvolumeup", render::glyph_image(Glyph::VolumeUp, FG, "PC")),
		("pcvolumedown", render::glyph_image(Glyph::VolumeDown, FG, "PC")),
		("pcmute", render::glyph_image(Glyph::Mute, FG, "PC")),
		("call", render::call_image(Some(&ringing))),
		("signal", render::signal_image(3, "LTE")),
		("sms", render::text_image("SMS", "Template", "")),
		("share", render::glyph_image(Glyph::Share, FG, "")),
		("ping", render::glyph_image(Glyph::Ping, FG, "")),
		(
			"nowplaying",
			render::now_playing_image(&NowPlaying {
				title: "Now Playing",
				artist: "KDE Connect",
				is_playing: true,
				progress: Some(0.4),
				show_text: true,
				..Default::default()
			}),
		),
	];
	let mut svgs: Vec<(&str, String)> = action_icons
		.into_iter()
		.map(|(name, uri)| (name, svg_of(&uri)))
		.collect();
	svgs.push(("plugin", brand_icon(true)));
	svgs.push(("category", brand_icon(false)));

	for (name, svg) in &svgs {
		for (suffix, size) in [("", 72), ("@2x", 144)] {
			let path = out_dir.join(format!("{name}{suffix}.png"));
			rasterise(svg, size, &options).save_png(&path).expect("write png");
			println!("wrote {}", path.display());
		}
	}
	// The catalogue wants a larger plugin icon for the marketplace listing.
	let path = out_dir.join("plugin@4x.png");
	rasterise(&brand_icon(true), 288, &options)
		.save_png(&path)
		.expect("write png");
	println!("wrote {}", path.display());
}
