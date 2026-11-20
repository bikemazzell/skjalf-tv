# Changelog

## Unreleased

- A Trailer button on the movie details screen. In Game Mode the trailer opens in Steam's browser over the app; on the desktop it opens in the default browser.
- The Main screen footer shows when your Xtream account expires and how many days remain. It is highlighted at 7 days or less.
- Fixed: the transport bar, the audio and subtitle panel, and the program list no longer hide subtitles. Subtitles move above them while they show.

## 0.2.0 - 2026-11-19

- Posters and covers on the movie and series details screens and on the Main rows, and channel logos in Live TV and the player.
- A movie or series without a poster shows a frame from the video. The details screen captures it once, only while nothing plays and only when your account has a free connection.
- Images are cached in `~/.cache/skjalf/art`, up to 300 MiB.
- Requests name the app with the User-Agent `SkjalfTV/0.2.0`. Settings has a new User Agent row for providers that serve only known players.
- The Settings screen is a centered column.
- Image requests go to the servers that your provider lists for posters and logos, which can be other servers than the provider. The Privacy section of the README explains this.

## 0.1.3 - 2026-11-19

- Audio and subtitle languages. Settings has three new rows: Audio Language, Subtitles (On or Off), and Subtitle Language. Every movie, episode, and channel tries these languages. A stream without your audio language plays its default audio. A stream without your subtitle language shows no subtitles.
- Subtitles are off until you turn them on in Settings or in the player.
- Change the audio track and the subtitles while you watch. Press View on the Steam Deck (O on a keyboard), or select the subtitle icon at the right end of the transport bar. The change lasts until the stream ends.
- After a track change, playback continues as soon as the new track arrives. It does not wait for the full "Buffer After a Stall" time.
- Start quits the app. The View button no longer quits.
- Fixed: when the video renderer is missing, a tap on the player area now shows the controls.

## 0.1.2 - 2026-11-17

- Buffering settings for slow connections: Buffer Before Playback and Buffer After a Stall. The buffer is a temporary file in the cache directory.
- Current emoji in channel and group names.
- Fixed: the disk buffer works with mpv 0.41.

## 0.1.1 - 2026-11-17

- Lists scroll to the focused row in one frame, without overshoot.
- The installer's TV hint points at the shortcut properties in Steam.

## 0.1.0 - 2026-11-16

- First public release: live TV, movies, and series from an Xtream Codes provider or an M3U playlist, with Steam Deck controls.
- One-line installer and uninstaller.
