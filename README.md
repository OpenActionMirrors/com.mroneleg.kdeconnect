# KDE Connect for OpenDeck

An [OpenAction](https://openaction.amankhanna.me/) plugin for
[OpenDeck](https://github.com/nekename/OpenDeck) that puts your
[KDE Connect](https://kdeconnect.kde.org/) devices on your Stream Deck: media
with album art, volume, battery, notifications, calls, and more for phones and
tablets, plus remote commands, locking and volume for other computers.

Linux only: it talks to the KDE Connect daemon over D-Bus.

## Keys

| Key | Shows | Press |
|---|---|---|
| Now Playing | Album art, scrolling title/artist, progress | Play/Pause (or Next, or nothing) |
| Play / Pause | Play or pause, matching the device | Play/Pause |
| Next / Previous Track | Next / previous | Skip |
| Volume Up / Down | Current volume | ± step (default 5%), repeats while held |
| Mute | Volume, or red "Muted" | Set volume to 0; press again to restore |
| Seek Back / Forward | Jump size | Jump N seconds (default 10) |
| Switch Player | Current player, "n of m" | Cycle the device's media players |
| Battery | Charge, colour-coded, bolt while charging | Refresh |
| Notifications | Count badge and newest app | Refresh |
| Find My Device | Phone with ringing waves | Ring the device |
| Send Clipboard | Clipboard | Send this computer's clipboard to the device |
| Incoming Call | Caller while ringing, "On call", or "Missed" | Dismiss |
| Signal | Cellular bars and network type | Refresh |
| SMS Template | Label and message | Arm; press again within 3 s to send |
| Share Link | Share icon and label | Open a URL on the device (or send text) |
| Ping | Speech bubble | Send a ping notification |
| Run Command (PC) | Command name | Run a command another computer offers |
| Lock / Unlock (PC) | Padlock, red when locked | Toggle the other computer's lock |
| PC Volume Up / Down | Output volume | ± step, repeats while held |
| PC Mute | Output volume, or red "Muted" | Toggle mute on that output |

Every key can be pinned to a device in its settings. "Auto" picks the first
reachable device that has that key's KDE Connect feature enabled, so a Battery
key finds your phone while a Lock key finds your other computer. A key pinned
to a device without the feature shows "Not supported". Media keys can also pin
a player; PC volume keys can pick an output (default: the computer's default).

Keys update within a fraction of a second of a change on the device: the plugin
listens for KDE Connect's D-Bus signals and also polls once a second.

### Limitations (from KDE Connect itself)

- Run Command only works with computers; Android doesn't offer commands to the desktop.
- Incoming Call can't silence the ringer (KDE Connect doesn't expose that over
  D-Bus), and KDE Connect doesn't report when a call ends, so "Incoming" turns
  into "Missed" after 45 s and "On call" clears after 30 minutes or when pressed.
- SMS sending goes through `kdeconnect-cli`, which KDE Connect installs.
- The device's media player decides whether volume and seek requests are honoured.

## Requirements

- Linux (x86_64 or aarch64) with KDE Connect running and at least one paired, reachable device.
- OpenDeck (or another OpenAction server).

## Install

Download `com.mroneleg.kdeconnect.streamDeckPlugin` from the
[latest release](https://github.com/mroneleg/kde-connect-opendeck/releases/latest)
and install it with OpenDeck's "Install from file". Then restart OpenDeck.

### From source

```sh
scripts/install.sh   # cargo build --release, package, copy into ~/.config/opendeck/plugins
```

Fully quit OpenDeck (tray icon → Quit) and start it again. Install a copy, not a
symlink: OpenDeck refuses to serve files from a symlinked plugin folder, so icons
and the settings panel wouldn't load.

## Development

```sh
cargo test                                         # unit tests
cargo clippy --all-targets -- -D warnings
cargo run --features icons --bin build-icons       # regenerate assets/icons with resvg
scripts/package.sh                                 # dist/com.mroneleg.kdeconnect.streamDeckPlugin
```

- `KDC_DRY_RUN=1` makes the plugin log every write (volume, ring, SMS, lock, …)
  instead of performing it, for testing against a real device.
- `KDC_DEBUG=1` turns on debug logging (OpenDeck saves plugin output in its logs folder).
- `examples/fake_kdeconnect.rs` is a fake KDE Connect daemon with one paired
  computer, for testing the computer-only keys on a private bus
  (`dbus-run-session -- …`).

Releases: bump the version in `Cargo.toml` and `assets/manifest.json`, then push
a `vX.Y.Z` tag. The release workflow builds both Linux targets on GitHub-hosted
runners and attaches the `.streamDeckPlugin` to a GitHub release.

## Notes

- AI-assisted: this plugin was written with [Claude Code](https://claude.com/claude-code).
- Not affiliated with or endorsed by KDE. "KDE Connect" names the software this plugin talks to.
- The icons are original; KDE Connect's own icon isn't used.

## License

[MIT](LICENSE)
