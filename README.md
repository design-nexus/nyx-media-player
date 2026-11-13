# Media Player

A video player for [Omarchy](https://omarchy.org). It keeps your movies and TV shows in
a library, remembers where you stopped, and plays them with mpv. It takes its colors
from your Omarchy theme and fits a half-screen tile.

## What it does

- **Library:** scans your video folders (MKV, MP4, WebM, AVI, MOV, WMV, TS, MPEG, OGV and
  more) into a local database. Later scans only read new or changed files, and it picks
  up videos added, removed or renamed within a minute.
  - **Home:** Continue watching as a list with the time left and Resume (and Remove
    from Continue watching), the next episode of each show you're following, and what
    was added lately beside them.
  - **Movies:** a poster grid, sorted by title, year, date added or rating, with
    watched and unwatched filters (both remembered). Each movie has a page with its details and file info.
  - **TV shows:** shows, seasons (with Play season) and episodes (with Mark watched up
    to here), found from file names
    (`Show S01E02.mkv`, `Show 1x02.mkv`) or folders (`Show/Season 1/02 - Title.mkv`).
  - **All videos** in one sortable table with how far you've got in each (filter by
    kind or to what you haven't watched; <kbd>Q</kbd> queues, <kbd>W</kbd> marks watched).
  - **Folders** as they're laid out on disk, listed in the sidebar: the path across the
    top, each file with its progress, and Show in its folder from any video's menu.
  - **Search** (<kbd>Ctrl</kbd>+<kbd>F</kbd>) across titles, shows, episodes and plots.
- **Details and pictures:** names from file and folder names, then Kodi-style `.nfo`
  files (`movie.nfo`, `tvshow.nfo`, `<name>.nfo`), then optionally
  [TMDB](https://www.themoviedb.org) with your own free API key. Posters and thumbnails
  come from images beside the videos (`poster.jpg`, `<name>-poster.jpg`,
  `<name>-thumb.jpg`, and `fanart.jpg` or `<name>-fanart.jpg` behind a movie or show's
  page), from TMDB, or a frame grabbed from the video. **Fix match**
  picks the right TMDB entry when the guess is wrong.
- **Playback** with mpv: hardware decoding, every format mpv plays, and speed from
  0.25× to 3× with the pitch kept.
  - **Resume:** each video starts where it was stopped and is marked watched near the
    end. Near the end (or once a Credits chapter starts) an Up next card counts down
    to the next episode, with Play now and Cancel.
  - **Seeking:** <kbd>←</kbd>/<kbd>→</kbd>, <kbd>0</kbd>–<kbd>9</kbd> for a tenth of the
    way through, two-finger swipes on a touchpad, and a seek bar that previews the
    frame under the pointer. Chapters show as ticks on the seek bar, in a Chapters menu,
    and with <kbd>Page Up</kbd>/<kbd>Page Down</kbd>. Click the length to see the time
    left; the player also says when the video will end.
  - **Subtitles:** picked up from files beside the video (`.srt`, `.ass`, `.vtt`…) or
    loaded by hand, with styled ASS, a delay you can nudge, a size setting and an
    optional dark background. They move up out of the way while the controls show.
    Each video remembers its subtitle and sound tracks.
  - **Picture:** override the shape (16:9, 4:3, 2.35:1…), fill the window, turn it, or
    deinterlace it.
  - **Sound:** a delay for videos whose sound is out of step, night mode to even out
    loud and quiet parts, and an optional boost up to 150%. The player bar shows which
    subtitles and sound track are on, and its sound button holds all of these.
  - Fullscreen, frame stepping, controls that fade away while the video plays, and
    <kbd>Ctrl</kbd>+<kbd>S</kbd> to save the frame on screen to your pictures folder.
  - **Web addresses** (<kbd>Ctrl</kbd>+<kbd>L</kbd>): streams, and video pages through
    yt-dlp when it's installed.
  - The screen stays awake while a video plays.
- **Drag and drop:** drop videos, folders or M3U playlists on the window to play them
  (hold <kbd>Shift</kbd> to add them to the queue).
- **Queue and playlists:** play next or add to the queue from any video's right-click
  menu, make playlists, drag videos onto them in the sidebar, and import or export M3U.
- **Equalizer:** ten bands and a preamp, with presets and your own saved ones. It opens from the player bar's sound button or Settings.
- **Media keys and the bar:** MPRIS, so media keys, `playerctl` and status bars control
  it and show what's playing.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/design-nexus/nyx-media-player/main/install.sh | bash -s -- media-player
```

This installs GTK 4, mpv and ffmpeg if they're missing, builds with Cargo, and installs
`nyx-media-player` to `~/.local/bin` along with a launcher entry. Add `-s -- --default`
after `bash` to also make it the default app for video files.

To remove it, run the same line with `uninstall.sh` in place of `install.sh`. Add
`-s -- --purge` to also remove its settings, library database and playlists. Your video
files are never touched.

## Usage

```
nyx-media-player [OPTIONS] [FILES…]
```

| Option | What |
| --- | --- |
| `FILES…` | Play these videos, folders or `.m3u` playlists |
| `--enqueue` | Add `FILES` to the queue instead |
| `--fullscreen` | Play `FILES` in fullscreen |
| `--section ID` | Open a page: `home`, `movies`, `tv`, `videos`, `folders`, `now-playing`, `queue`, `equalizer`, `settings`; or a show (`tv:NAME`) or a movie (`movie:PATH`) |
| `--toggle` | Close the window if it's open, otherwise open it |
| `--play-pause`, `--play`, `--pause`, `--stop`, `--next`, `--previous` | Control playback without raising the window |

| Key | What |
| --- | --- |
| <kbd>Space</kbd> | Play or pause |
| <kbd>←</kbd> / <kbd>→</kbd> | Back / forward 5 seconds (<kbd>Shift</kbd>: 30 seconds) |
| <kbd>0</kbd>–<kbd>9</kbd> | Jump to that tenth of the video |
| <kbd>Page Up</kbd> / <kbd>Page Down</kbd> | Previous / next chapter |
| <kbd>Ctrl</kbd>+<kbd>←</kbd> / <kbd>→</kbd> | Previous / next video |
| <kbd>↑</kbd> / <kbd>↓</kbd> | Louder / quieter |
| <kbd>F</kbd>, <kbd>Esc</kbd> | Fullscreen, leave fullscreen |
| <kbd>Alt</kbd>+<kbd>←</kbd> | Back (also the mouse's back button); <kbd>Esc</kbd> closes a movie or show |
| <kbd>M</kbd> | Mute |
| <kbd>S</kbd> / <kbd>A</kbd> | Next subtitles / next sound track |
| <kbd>Z</kbd> / <kbd>X</kbd> | Subtitles earlier / later |
| <kbd>Ctrl</kbd>+<kbd>−</kbd> / <kbd>+</kbd> | Sound earlier / later |
| <kbd>N</kbd> | Night mode |
| <kbd>[</kbd> / <kbd>]</kbd> | Slower / faster |
| <kbd>,</kbd> / <kbd>.</kbd> | Back / forward one frame |
| <kbd>Ctrl</kbd>+<kbd>O</kbd> | Open files |
| <kbd>Ctrl</kbd>+<kbd>L</kbd> | Open a web address |
| <kbd>Ctrl</kbd>+<kbd>S</kbd> | Save the frame as a picture |
| <kbd>F1</kbd> or <kbd>?</kbd> | All keyboard shortcuts |
| <kbd>Ctrl</kbd>+<kbd>F</kbd> | Search the library |
| <kbd>Ctrl</kbd>+<kbd>Q</kbd> | Close |

On a touchpad, swipe sideways over the video to seek and up or down to change the
volume. Click to pause; double-click for fullscreen.

## Files

| Path | What |
| --- | --- |
| `~/.config/nyx-media-player/settings.toml` | Preferences: library folders, playback, subtitles, TMDB, equalizer, theme |
| `~/.config/nyx-media-player/themes/*.toml` | Your own themes |
| `~/.local/share/nyx-media-player/library.db` | The library, watch progress, playlists and TMDB answers |
| `~/.local/share/nyx-media-player/state.json` | The queue, for resuming |
| `~/.cache/nyx-media-player/art/` | Posters and frames, scaled |
| `~/.cache/nyx-media-player/scrub/` | Seek bar previews |

This product uses the TMDB API but is not endorsed or certified by TMDB.
