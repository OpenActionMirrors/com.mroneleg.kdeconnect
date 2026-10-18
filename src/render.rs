//! Key images rendered as SVG data URIs (OpenDeck rasterises them with resvg).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

const SIZE: f64 = 144.0;
const PAD: f64 = 8.0;
const MARQUEE_GAP: f64 = 36.0;

pub const BG: &str = "#18181b";
pub const FG: &str = "#fafafa";
pub const MUTED: &str = "#a1a1aa";
pub const DIM: &str = "#71717a";
pub const ACCENT: &str = "#3daee9"; // KDE blue
pub const GREEN: &str = "#22c55e";
pub const AMBER: &str = "#f59e0b";
pub const RED: &str = "#ef4444";

const TITLE_SIZE: f64 = 19.0;
const ARTIST_SIZE: f64 = 15.0;

/// A named glyph drawn on the 144x144 key canvas. Stroked glyphs use `{c}` for
/// their stroke colour; everything else takes the group fill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
	Play,
	Pause,
	Next,
	Previous,
	VolumeUp,
	VolumeDown,
	Speaker,
	Mute,
	FindMyPhone,
	Clipboard,
	Bell,
	Locked,
	Unlocked,
	Share,
	Ping,
	Handset,
	Disconnected,
}

const SPEAKER_BODY: &str = r#"<path d="M26 60 H44 L66 40 V104 L44 84 H26 Z"/>"#;

impl Glyph {
	fn markup(self) -> String {
		match self {
			Glyph::Play => r#"<path d="M54 40 L106 72 L54 104 Z"/>"#.into(),
			Glyph::Pause => r#"<rect x="46" y="40" width="18" height="64" rx="3"/><rect x="80" y="40" width="18" height="64" rx="3"/>"#.into(),
			Glyph::Next => r#"<path d="M38 40 L82 72 L38 104 Z"/><rect x="88" y="40" width="14" height="64" rx="3"/>"#.into(),
			Glyph::Previous => r#"<rect x="42" y="40" width="14" height="64" rx="3"/><path d="M106 40 L62 72 L106 104 Z"/>"#.into(),
			Glyph::VolumeUp => format!(
				r#"{SPEAKER_BODY}<rect x="80" y="68" width="36" height="9" rx="3"/><rect x="93.5" y="54.5" width="9" height="36" rx="3"/>"#
			),
			Glyph::VolumeDown => format!(r#"{SPEAKER_BODY}<rect x="80" y="68" width="36" height="9" rx="3"/>"#),
			Glyph::Speaker => format!(
				r#"{SPEAKER_BODY}<path d="M82 56 Q92 72 82 88 M96 46 Q112 72 96 98" stroke="{{c}}" stroke-width="7" stroke-linecap="round" fill="none"/>"#
			),
			Glyph::Mute => format!(
				r#"{SPEAKER_BODY}<path d="M84 58 L112 86 M112 58 L84 86" stroke="{{c}}" stroke-width="9" stroke-linecap="round" fill="none"/>"#
			),
			Glyph::FindMyPhone => concat!(
				r#"<rect x="52" y="32" width="40" height="76" rx="8" fill="none" stroke="{c}" stroke-width="7"/><circle cx="72" cy="96" r="4"/>"#,
				r#"<path d="M38 54 Q31 70 38 86 M106 54 Q113 70 106 86 M26 44 Q14 70 26 96 M118 44 Q130 70 118 96" stroke="{c}" stroke-width="6" fill="none" stroke-linecap="round"/>"#,
			)
			.into(),
			Glyph::Clipboard => concat!(
				r#"<rect x="40" y="34" width="64" height="80" rx="8" fill="none" stroke="{c}" stroke-width="7"/><rect x="56" y="26" width="32" height="16" rx="4"/>"#,
				r#"<path d="M54 66 H90 M54 82 H90 M54 98 H78" stroke="{c}" stroke-width="6" stroke-linecap="round"/>"#,
			)
			.into(),
			Glyph::Bell => r#"<path d="M72 30 C54 30 46 44 46 62 V80 L36 94 H108 L98 80 V62 C98 44 90 30 72 30 Z"/><path d="M61 100 A11 11 0 0 0 83 100 Z"/>"#.into(),
			Glyph::Locked => r#"<path d="M50 64 V50 A22 22 0 0 1 94 50 V64" fill="none" stroke="{c}" stroke-width="9"/><rect x="38" y="62" width="68" height="50" rx="8"/>"#.into(),
			Glyph::Unlocked => r#"<path d="M50 64 V50 A22 22 0 0 1 94 50 V54" fill="none" stroke="{c}" stroke-width="9" transform="translate(22 -8)"/><rect x="38" y="62" width="68" height="50" rx="8"/>"#.into(),
			Glyph::Share => concat!(
				r#"<path d="M72 28 V82 M52 48 L72 28 L92 48" fill="none" stroke="{c}" stroke-width="9" stroke-linecap="round" stroke-linejoin="round"/>"#,
				r#"<path d="M42 66 H36 V108 H108 V66 H102" fill="none" stroke="{c}" stroke-width="8" stroke-linecap="round" stroke-linejoin="round"/>"#,
			)
			.into(),
			Glyph::Ping => concat!(
				r#"<path d="M30 36 H114 A8 8 0 0 1 122 44 V88 A8 8 0 0 1 114 96 H64 L44 112 V96 H30 A8 8 0 0 1 22 88 V44 A8 8 0 0 1 30 36 Z"/>"#,
				r##"<circle cx="50" cy="66" r="7" fill="#18181b"/><circle cx="72" cy="66" r="7" fill="#18181b"/><circle cx="94" cy="66" r="7" fill="#18181b"/>"##,
			)
			.into(),
			Glyph::Handset => r#"<path d="M44 30 C36 30 30 38 32 48 C38 76 62 100 92 110 C102 112 110 106 110 98 L110 86 L88 78 L80 88 C68 82 58 72 52 60 L62 52 L54 30 Z"/>"#.into(),
			Glyph::Disconnected => r#"<path d="M44 44 L100 100 M100 44 L44 100" stroke="{c}" stroke-width="12" stroke-linecap="round" fill="none"/>"#.into(),
		}
	}
}

pub fn escape_xml(value: &str) -> String {
	value
		.replace('&', "&amp;")
		.replace('<', "&lt;")
		.replace('>', "&gt;")
		.replace('"', "&quot;")
		.replace('\'', "&apos;")
}

/// Number formatting that matches JavaScript template literals for the values
/// used here: integers print without a decimal point.
fn num(value: f64) -> String {
	if value.fract() == 0.0 && value.abs() < 1e15 {
		format!("{}", value as i64)
	} else {
		format!("{value}")
	}
}

pub fn to_data_uri(svg: &str) -> String {
	format!("data:image/svg+xml;base64,{}", STANDARD.encode(svg))
}

fn wrap_svg(body: &str) -> String {
	format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="144" height="144" viewBox="0 0 144 144">{body}</svg>"#)
}

fn background() -> String {
	format!(r#"<rect width="144" height="144" fill="{BG}"/>"#)
}

fn char_len(text: &str) -> usize {
	text.chars().count()
}

// Rough average glyph width for sans-serif; good enough to decide when to scroll.
fn estimate_text_width(text: &str, font_size: f64) -> f64 {
	char_len(text) as f64 * font_size * 0.56
}

pub fn truncate(text: &str, max: usize) -> String {
	if char_len(text) > max {
		let kept: String = text.chars().take(max - 1).collect();
		format!("{kept}…")
	} else {
		text.to_string()
	}
}

fn fit_font_size(text: &str, max: f64) -> f64 {
	let width = SIZE - PAD * 2.0;
	(width / (char_len(text).max(1) as f64 * 0.56))
		.floor()
		.min(max)
		.max(11.0)
}

fn caption(label: &str) -> String {
	if label.is_empty() {
		return String::new();
	}
	format!(
		r#"<text x="72" y="132" fill="{MUTED}" font-family="sans-serif" font-size="14" text-anchor="middle">{}</text>"#,
		escape_xml(&truncate(label, 18))
	)
}

/// A glyph on the dark key background with an optional caption.
pub fn glyph_image(glyph: Glyph, color: &str, label: &str) -> String {
	let markup = glyph.markup().replace("{c}", color);
	to_data_uri(&wrap_svg(&format!(
		r#"{}<g fill="{color}">{markup}</g>{}"#,
		background(),
		caption(label)
	)))
}

/// Circular arrow with the jump size in the middle.
pub fn seek_image(forward: bool, seconds: &str, color: &str) -> String {
	let arrow = format!(
		r#"<path d="M72 36 A34 34 0 1 1 41 56" fill="none" stroke="{color}" stroke-width="8" stroke-linecap="round"/><path d="M58 36 L76 24 L76 48 Z" fill="{color}"/>"#
	);
	// The arrow is drawn turning backwards; mirror it for forward.
	let arc = if forward {
		format!(r#"<g transform="translate(144 0) scale(-1 1)">{arrow}</g>"#)
	} else {
		arrow
	};
	to_data_uri(&wrap_svg(&format!(
		r#"{}{arc}<text x="72" y="80" fill="{color}" font-family="sans-serif" font-size="24" font-weight="bold" text-anchor="middle">{}</text>"#,
		background(),
		escape_xml(seconds)
	)))
}

/// Battery gauge with fill level, percentage, and a bolt while charging.
/// `charge` is None (or negative) when unknown.
pub fn battery_image(charge: Option<i32>, is_charging: bool) -> String {
	let known = matches!(charge, Some(c) if c >= 0);
	let level = if known { charge.unwrap_or(0).clamp(0, 100) } else { 0 };
	let color = if !known {
		MUTED
	} else if is_charging {
		GREEN
	} else if level <= 20 {
		RED
	} else if level <= 40 {
		AMBER
	} else {
		FG
	};
	let fill_width = 64.0 * f64::from(level) / 100.0;
	let bolt = if is_charging {
		format!(
			r#"<path d="M76 46 L60 70 H71 L66 88 L84 62 H73 Z" fill="{FG}" stroke="{BG}" stroke-width="3" stroke-linejoin="round"/>"#
		)
	} else {
		String::new()
	};
	let fill = if fill_width > 0.0 {
		format!(r#"<rect x="36" y="52" width="{fill_width:.1}" height="28" rx="3" fill="{color}"/>"#)
	} else {
		String::new()
	};
	let label = if known { format!("{level}%") } else { "—".into() };
	to_data_uri(&wrap_svg(&format!(
		r#"{}<rect x="30" y="46" width="76" height="40" rx="7" fill="none" stroke="{color}" stroke-width="6"/><rect x="108" y="57" width="8" height="18" rx="3" fill="{color}"/>{fill}{bolt}<text x="72" y="124" fill="{FG}" font-family="sans-serif" font-size="26" font-weight="bold" text-anchor="middle">{label}</text>"#,
		background()
	)))
}

/// Bell with an unread badge and the newest notification's app underneath.
pub fn notifications_image(count: usize, latest_app: &str) -> String {
	let color = if count > 0 { FG } else { MUTED };
	let badge_text = if count > 99 {
		"99+".to_string()
	} else {
		count.to_string()
	};
	let badge = if count > 0 {
		format!(
			r##"<circle cx="102" cy="40" r="20" fill="{RED}" stroke="{BG}" stroke-width="4"/><text x="102" y="48" fill="#fff" font-family="sans-serif" font-size="{}" font-weight="bold" text-anchor="middle">{badge_text}</text>"##,
			if badge_text.len() > 2 { 15 } else { 22 }
		)
	} else {
		String::new()
	};
	let label = if count > 0 { latest_app } else { "No notifications" };
	to_data_uri(&wrap_svg(&format!(
		r#"{}<g fill="{color}" transform="translate(0 -6)">{}</g>{badge}{}"#,
		background(),
		Glyph::Bell.markup(),
		caption(label)
	)))
}

/// Phone signal bars (strength 0-4, -1 unknown) and the network type.
pub fn signal_image(strength: i32, network_type: &str) -> String {
	let known = strength >= 0;
	let bars: String = (0..4)
		.map(|i| {
			let height = 18 + i * 16;
			let lit = known && i < strength;
			format!(
				r#"<rect x="{}" y="{}" width="16" height="{height}" rx="3" fill="{}"/>"#,
				30 + i * 22,
				96 - height,
				if lit { FG } else { "#3f3f46" }
			)
		})
		.collect();
	let label = if !known {
		"No signal"
	} else if network_type.is_empty() {
		"Signal"
	} else {
		network_type
	};
	to_data_uri(&wrap_svg(&format!(
		r#"{}{bars}<text x="72" y="128" fill="{}" font-family="sans-serif" font-size="20" font-weight="bold" text-anchor="middle">{}</text>"#,
		background(),
		if known { FG } else { MUTED },
		escape_xml(label)
	)))
}

/// What telephony last reported for a device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallInfo {
	pub event: String,
	pub number: String,
	pub name: String,
}

/// Incoming / active / missed call, or idle.
pub fn call_image(call: Option<&CallInfo>) -> String {
	let state = call.and_then(|c| match c.event.as_str() {
		"ringing" => Some((GREEN, "Incoming")),
		"talking" => Some((ACCENT, "On call")),
		"missedCall" => Some((RED, "Missed")),
		_ => None,
	});
	let (Some(call), Some((color, heading))) = (call, state) else {
		return to_data_uri(&wrap_svg(&format!(
			r#"{}<g fill="{MUTED}" transform="translate(14 0) scale(0.8)">{}</g>{}"#,
			background(),
			Glyph::Handset.markup(),
			caption("No calls")
		)));
	};
	let who = if !call.name.is_empty() {
		call.name.as_str()
	} else if !call.number.is_empty() {
		call.number.as_str()
	} else {
		"Unknown"
	};
	let size = fit_font_size(who, 20.0);
	to_data_uri(&wrap_svg(&format!(
		r#"{}<g fill="{color}" transform="translate(40 10) scale(0.45)">{}</g><text x="72" y="84" fill="{color}" font-family="sans-serif" font-size="16" font-weight="bold" text-anchor="middle">{heading}</text><text x="72" y="{}" fill="{FG}" font-family="sans-serif" font-size="{}" font-weight="bold" text-anchor="middle">{}</text>"#,
		background(),
		Glyph::Handset.markup(),
		num(108.0 + size / 3.0),
		num(size),
		escape_xml(&truncate(who, 16))
	)))
}

/// Small heading, a large fitted value, and an optional footer.
pub fn text_image(heading: &str, value: &str, footer: &str) -> String {
	let size = fit_font_size(value, 28.0);
	to_data_uri(&wrap_svg(&format!(
		r#"{}<text x="72" y="38" fill="{MUTED}" font-family="sans-serif" font-size="16" font-weight="bold" text-anchor="middle">{}</text><text x="72" y="{}" fill="{FG}" font-family="sans-serif" font-size="{}" font-weight="bold" text-anchor="middle">{}</text>{}"#,
		background(),
		escape_xml(heading),
		num(80.0 + size / 3.0),
		num(size),
		escape_xml(value),
		caption(footer)
	)))
}

/// One line of text clipped to the key width. If it doesn't fit, it is drawn
/// twice with a gap so `offset` can scroll it as a seamless loop.
fn marquee_line(text: &str, y: f64, font_size: f64, color: &str, weight: &str, offset: f64, clip_id: &str) -> String {
	let width = estimate_text_width(text, font_size);
	let avail = SIZE - PAD * 2.0;
	let attrs = format!(
		r#"fill="{color}" font-family="sans-serif" font-size="{}" font-weight="{weight}""#,
		num(font_size)
	);
	let text = escape_xml(text);
	if width <= avail {
		return format!(
			r#"<text x="72" y="{}" {attrs} text-anchor="middle">{text}</text>"#,
			num(y)
		);
	}
	let period = width + MARQUEE_GAP;
	let shift = offset % period;
	let x1 = PAD - shift;
	let x2 = x1 + period;
	format!(
		r#"<clipPath id="{clip_id}"><rect x="{}" y="{}" width="{}" height="{}"/></clipPath><g clip-path="url(#{clip_id})"><text x="{}" y="{}" {attrs}>{text}</text><text x="{}" y="{}" {attrs}>{text}</text></g>"#,
		num(PAD),
		num(y - font_size),
		num(avail),
		num(font_size * 1.4),
		num(x1),
		num(y),
		num(x2),
		num(y)
	)
}

fn needs_scroll(text: &str, font_size: f64) -> bool {
	estimate_text_width(text, font_size) > SIZE - PAD * 2.0
}

/// Inputs for the Now Playing key.
#[derive(Debug, Default)]
pub struct NowPlaying<'a> {
	pub title: &'a str,
	pub artist: &'a str,
	pub is_playing: bool,
	/// 0..1, or None to hide the progress bar.
	pub progress: Option<f64>,
	pub art_data_uri: Option<&'a str>,
	pub offset: f64,
	pub show_text: bool,
}

/// Album art (if any) with title / artist over a gradient and a progress bar.
pub fn now_playing_image(np: &NowPlaying) -> String {
	let mut parts = vec![background()];
	match np.art_data_uri {
		Some(uri) => parts.push(format!(
			r#"<image href="{uri}" x="0" y="0" width="144" height="144" preserveAspectRatio="xMidYMid slice"/>"#
		)),
		None => parts.push(format!(
			r##"<g fill="#3f3f46" transform="translate(0 -18)"><circle cx="72" cy="72" r="34"/><circle cx="72" cy="72" r="8" fill="{BG}"/></g>"##
		)),
	}
	if np.show_text {
		parts.push(
			r##"<defs><linearGradient id="fade" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#000" stop-opacity="0"/><stop offset="1" stop-color="#000" stop-opacity="0.85"/></linearGradient></defs>"##.into(),
		);
		parts.push(r#"<rect x="0" y="64" width="144" height="80" fill="url(#fade)"/>"#.into());
		let title = if np.title.is_empty() {
			"Nothing playing"
		} else {
			np.title
		};
		parts.push(marquee_line(
			title,
			110.0,
			TITLE_SIZE,
			FG,
			"bold",
			np.offset,
			"clipTitle",
		));
		if !np.artist.is_empty() {
			parts.push(marquee_line(
				np.artist,
				129.0,
				ARTIST_SIZE,
				"#d4d4d8",
				"normal",
				np.offset,
				"clipArtist",
			));
		}
	}
	if !np.is_playing && !np.title.is_empty() {
		parts.push(r##"<rect width="144" height="144" fill="#000" fill-opacity="0.35"/>"##.into());
		parts.push(
			r##"<g fill="#fff" fill-opacity="0.9"><rect x="112" y="10" width="7" height="22" rx="2"/><rect x="124" y="10" width="7" height="22" rx="2"/></g>"##.into(),
		);
	}
	if let Some(progress) = np.progress {
		let p = progress.clamp(0.0, 1.0);
		parts.push(r##"<rect x="0" y="139" width="144" height="5" fill="#000" fill-opacity="0.6"/>"##.into());
		parts.push(format!(
			r#"<rect x="0" y="139" width="{:.1}" height="5" fill="{ACCENT}"/>"#,
			SIZE * p
		));
	}
	to_data_uri(&wrap_svg(&parts.concat()))
}

/// Whether the Now Playing key needs the marquee timer for this track.
pub fn now_playing_scrolls(title: &str, artist: &str) -> bool {
	needs_scroll(title, TITLE_SIZE) || needs_scroll(artist, ARTIST_SIZE)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn decode(uri: &str) -> String {
		let b64 = uri.strip_prefix("data:image/svg+xml;base64,").expect("svg data uri");
		String::from_utf8(STANDARD.decode(b64).unwrap()).unwrap()
	}

	#[test]
	fn escapes_markup_in_labels() {
		let svg = decode(&glyph_image(Glyph::Play, FG, "<Tom & Jerry>"));
		assert!(svg.contains("&lt;Tom &amp; Jerry&gt;"));
	}

	#[test]
	fn truncates_long_captions_with_ellipsis() {
		assert_eq!(truncate("Google Play services", 18), "Google Play servi…");
		assert_eq!(truncate("short", 18), "short");
	}

	#[test]
	fn short_titles_are_centred_and_long_ones_scroll() {
		assert!(!now_playing_scrolls("Super Unison", "Drive Like Jehu"));
		assert!(now_playing_scrolls("Wilma's Rainbow (Album Version)", "Helmet"));
		let svg = decode(&now_playing_image(&NowPlaying {
			title: "Wilma's Rainbow (Album Version)",
			show_text: true,
			..Default::default()
		}));
		assert!(svg.contains("clipTitle"));
	}

	#[test]
	fn battery_colours_follow_level_and_charging() {
		assert!(decode(&battery_image(Some(12), false)).contains(RED));
		assert!(decode(&battery_image(Some(35), false)).contains(AMBER));
		assert!(decode(&battery_image(Some(100), true)).contains(GREEN));
		assert!(decode(&battery_image(None, false)).contains("—"));
	}

	#[test]
	fn call_image_shows_caller_or_idle() {
		let call = CallInfo {
			event: "ringing".into(),
			number: "+15550100".into(),
			name: "".into(),
		};
		let svg = decode(&call_image(Some(&call)));
		assert!(svg.contains("Incoming") && svg.contains("+15550100"));
		assert!(decode(&call_image(None)).contains("No calls"));
	}

	#[test]
	fn numbers_format_like_javascript() {
		assert_eq!(num(26.0), "26");
		assert_eq!(num(8.5), "8.5");
	}
}
