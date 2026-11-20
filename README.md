<p align="center"><img src="packaging/icons/skjalf-256.png" width="128" alt="Skjalf TV icon"></p>

# Skjalf TV

An IPTV player for the Steam Deck and Linux. It plays live TV, movies, and series from your Xtream Codes account or M3U playlist, with the Deck's buttons, touch, or a keyboard.

Skjalf TV includes no channels. You need your own IPTV subscription or playlist.

<a href="https://alternativeto.net/software/skjalf-tv/about/?utm_source=badge&utm_medium=referral"><img src="https://alternativeto.net/static/badges/badge-compact-color.svg" alt="Skjalf TV | AlternativeTo" width="171" height="55"></a>

![The main screen with Continue Watching, Latest Movies, and Latest Series](docs/screenshots/home.png)

| Live TV | Player | Audio and subtitles |
|---|---|---|
| ![Live TV channel list](docs/screenshots/live-tv.png) | ![Live player with the transport bar](docs/screenshots/player.png) | ![The audio and subtitle panel](docs/screenshots/tracks.png) |

## Install on a Steam Deck

1. Hold the power button and select Switch to Desktop.
2. Open Konsole and run:

   ```sh
   curl -fsSL https://github.com/bikemazzell/skjalf-tv/releases/latest/download/install.sh | sh
   ```

3. Select Return to Gaming Mode. Skjalf TV is in your library under Non-Steam.
4. Start it, choose Xtream Codes or M3U, and enter your provider details.

To update, run the same command again. To uninstall, use `uninstall.sh` in place of `install.sh`. Add `-s -- --purge` after `sh` to also delete your settings and saved login.

## Features

- Live TV with groups, search, favorites, and channel numbers.
- Movies and series with posters, details, and resume. It stays fast with more than 180,000 titles.
- Audio and subtitle languages set once in Settings, and a panel to switch tracks while you watch.
- Buffering settings for slow connections.

## Good to know

- The installer closes Steam for a moment to add its shortcut. It checks the hash of every download.
- On a TV, set "Game resolution" to Native in the shortcut's properties, and start the app after you connect the TV.
- If your provider refuses the app, set User Agent in Settings to a player name that your provider accepts.
- If video keeps stopping to buffer, raise the two buffer settings in Settings.
- A title without a poster shows a frame from its video. Capturing it uses one provider connection for a few seconds, and only when one is free.

## Controls

| Steam Deck | Keyboard | Action |
|---|---|---|
| D-pad | Arrow keys | Move |
| A | Enter | Select |
| B | Esc | Back |
| X | E | Search, or program information in the player |
| Y | F | Favorite |
| L1 and R1 | Page Up and Page Down | Page through lists, or change the channel |
| View | O | Audio and subtitles in the player |

## Privacy

Skjalf TV keeps your settings, favorites, watch progress, and cached catalogs in `~/.config/skjalf` (or `$XDG_CONFIG_HOME/skjalf`). These files contain your provider address and credentials. The app creates them readable only by your user. Treat the whole directory as private.

The app sends data only to your provider and to the image servers that your provider lists for posters and logos. These can be other servers, for example an image service. An image request tells that server your IP address and the app's User-Agent, but not your provider credentials. The app does not write your credentials to logs or error messages.

## Build

You need Rust 1.90 or later. A development script downloads libmpv packages into `.deps/mpv`. These libraries are for builds and tests on the workstation only, not for SteamOS.

```sh
./scripts/fetch-mpv-deps.sh
export MPV_LIBRARY_DIR="$PWD/.deps/mpv/usr/lib/x86_64-linux-gnu"
export LIBRARY_PATH="$MPV_LIBRARY_DIR"
export LD_LIBRARY_PATH="$MPV_LIBRARY_DIR"
export PKG_CONFIG_PATH="$MPV_LIBRARY_DIR/pkgconfig"
cargo build --release -p skjalf
```

The binary is `target/release/skjalf`. To install it on a Deck, copy the source tree and the binary there, close Steam, and run `packaging/install-steamos.sh /absolute/path/to/skjalf`.

## Tests

```sh
cargo test --workspace --locked
cargo test -p skjalf --test mpv_integration --locked -- --ignored --test-threads=1
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

The `mpv_integration` tests play generated clips through the real libmpv. The packaging tests need a release binary and the libmpv directory:

```sh
SKJALF_TEST_BINARY="$PWD/target/release/skjalf" \
SKJALF_TEST_MPV_LIB_DIR="$MPV_LIBRARY_DIR" \
SKJALF_TEST_NETWORK=1 \
python3 -m unittest discover -s packaging/tests -v
```

Without `SKJALF_TEST_NETWORK=1`, the test that downloads the SteamOS runtime is skipped. The packaging tests write only to temporary directories.

Tests never start the real Steam client. To check this, put a fake `steam` script that logs its arguments first on `PATH`. Then run the tests and make sure that the log is empty.

## Credits

The icon shows a TV whose antenna is the rune Algiz, with Odin's eye on the screen. The name comes from Hliðskjálf, Odin's high seat. The app includes the Noto Emoji font under the SIL Open Font License (`crates/app/assets/fonts/NotoEmoji-OFL.txt`).

## License

MIT. See [LICENSE](LICENSE). [CHANGELOG.md](CHANGELOG.md) lists the changes in each release.
