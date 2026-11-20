# Changelog

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
