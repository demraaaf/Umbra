# Changelog

## 1.0.0

The first full release.

- Settings start simple: each page shows the choices most people need, and Show advanced
  settings (at the top of Settings) shows the rest. A search always finds every setting. The
  switch points itself out until it has been tried once.
- The Video page, without advanced settings, offers Quality (Smaller files, Balanced, Best),
  Frame rate, Resolution and where clips play (Everywhere, or Smaller files) instead of
  codecs and bitrates. A quality level fits any size and frame rate, and keeps its quality
  when either changes.
- The first setup has a Clip quality step: four questions and a 10-second test of the
  graphics card pick the video settings, so nobody has to choose a codec. Its result goes to
  the stock FPS and Story presets too.
- One Overlays page for the camera, the logo and the keyboard and mouse, placed together on
  one preview. Auto-clip has a section per game.
- The mouse in the keyboard overlay looks like a mouse: two buttons filled when held, the
  wheel in a slot with an arrow the way it turns, the side buttons on its flank.
- Umbra asks Windows to leave out the yellow border it draws around a recorded window or
  screen (Windows 11; Windows 10 always shows it).
- Fixed: a game that closed its window and opened a new one (switching to fullscreen, a
  launcher handing over) froze the recording on its last picture until the game closed. It
  now carries on with the new window, and a clip saved meanwhile still has what came before.
- Fixed: webcams filmed in MJPEG showed a frame from 12 frames earlier every 24 frames
  on NVIDIA graphics cards. MJPEG is decoded on the processor now.
- Fixed: with Skip the Recycle Bin on, one clip open in another program stopped the storage
  limit from deleting anything. Every file is tried now, and clips that went also leave
  their albums when others could not go.
- Fixed: renaming a clip to change only capital letters said the name was taken.
- A settings or albums file Umbra cannot read (written by a newer version, or edited by hand)
  is kept as `.unreadable` before it is replaced, instead of being lost.

- Burn in, from the library: right-click a clip (or a selection) and draw its camera, keyboard
  and mouse, logo or all of them into the picture for good, in the background, one clip after
  another, with a progress card and Stop. The video is encoded again with the clip's own codec
  at about its own quality; every audio track is copied as it is, and what is not burned in
  stays a layer. The new file is `<name> (burned).mp4` next to the clip, or, with Settings,
  Player and editor, "Burn in replaces the original", takes its place once it checks out.
  Clips show small icons for their camera, input and logo, and the library filters by
  "Has camera" and "Has input".
- Software cameras: OBS's virtual camera and other cameras only DirectShow knows are listed
  with the webcams (marked DirectShow) and recorded like them. A webcam both list shows once.
  One that stops sending (OBS closed) leaves a gap in the clip and is picked up again when it
  sends.
- Keyboard and mouse overlay (Settings, Keyboard and mouse; off by default): records which
  keys and buttons you press, to show them over clips in the player and the editor and to draw
  them in on export. WASD, a compact keyboard, the mouse with its wheel and movement, and a
  line of the keys pressed last; size, opacity and highlight colour, and per clip on or off and
  moved in the editor. What is kept depends on the program in front, by rules you can reorder:
  by default everything in games, no keys in chat apps, nothing in password managers, and in
  browsers and everything else the keys that type text only as a dot. Password fields are never
  recorded, a game's chat is hidden until Enter or Esc, and nothing is kept while Umbra is in
  front. Input is read the way Windows offers it to every program (no hooks) and kept in a file
  next to each clip, never in the video, so a shared clip carries none of it. A hotkey (none by
  default) and a panel widget switch it.
- Auto-clip (Settings, Auto-clip; off by default): saves a clip by itself when something
  happens in a game, named after it (`… - Triple kill, Collateral.mp4`), with every audio
  track like a hotkey clip. First game: Counter-Strike 2, through Valve's Game State
  Integration. Kills are counted per round: clip every kill, or only rounds with a 2K, 3K, 4K
  or an ace; the clip covers the round's kills and is saved when the round ends. Special kills
  (headshot, knife, Zeus, collateral, blind) are always named in the clip and can make a clip
  by themselves; deaths and round wins too. A game-mode filter (Competitive and Premier,
  Wingman, Casual, Deathmatch, Arms Race, others). One button writes the game's config file.
  Its own sound volume (0 is silent), and an event log for tuning.
- Recording panel on the right: start and stop, switch between monitors and windows with
  small previews, the microphone and its push-to-talk key, the hotkeys, and what Umbra itself
  uses (CPU, memory, GPU, video encoder). The status line at the bottom left is gone.
- Push-to-talk for the microphone, with its own key.
- Screenshots with their own hotkey (PNG or JPEG), shown next to the clips.
- Albums: a new tab, add clips from the library or the editor, and optionally hide an album's
  clips from the main list.
- Select clips with the middle mouse button, or hold it and sweep over clips; delete the
  selection or add it to an album.
- Per-track volume from 0 to 200 % in the editor, heard in the preview. The combined first
  track is rebuilt from its parts on export, so a volume change reaches every player.
- Presets and plain explanations for video and audio bitrate, codec and the other less common
  settings; an estimate of memory, clip size, CPU and encoder load for the current settings.
- Settings folder shown under Storage, with a button to open it.
- Discord gets its own audio track by default on new installs.
- Both sidebars can be collapsed (Ctrl+B and Ctrl+Alt+B).
- A player: one click on a clip plays it, a double-click opens the editor. The arrow keys skip
  a set number of seconds (Settings, Player), Shift and an arrow steps one frame, Ctrl and an
  arrow goes to the previous or next clip. Plays high frame rates as recorded, up to 360 fps.
- In the editor, Shift and the arrow keys (or Shift and a drag on the timeline) select a part;
  Delete cuts it out and the export joins the rest, frame-exact, on every audio track.
- Waveforms of every audio track under the timeline when the window has room for them.
- A Help page in Settings with every mouse and keyboard control.
- Double-clicking the title bar maximizes the window. The encoder figures are gone from the
  recording panel.
- Fixed: removing or turning down an app with its own track (Spotify, Discord) left it in "Other
  apps" and the combined track. Umbra now finds the app inside "Other apps" (the two captures
  are a few milliseconds apart, a different amount each time) and takes it out there too, on
  export and in the editor's preview; it ends up about 25 dB quieter. Works on existing clips.
- Resting the pointer on a clip plays it in its thumbnail.
- Favorites: a built-in album that cannot be deleted, filled from a heart on each clip, in the
  list view and in the clip's header.
- Properties for every clip (right-click menu or the header): file, video, average bitrate,
  and for new clips how they were recorded: the game and program detected and how, the
  window, capture source, encoder, preset and rate control.
- Clip cards have a border and a darker information strip, so the thumbnails stand out.
- A right-click menu on clips: play, edit, favorite, show in folder, properties, delete.
- The recording panel folds away while the editor is open and comes back after, unless it was
  already folded.
- "Other apps" is now recorded without the apps that have their own track: every other program
  is captured on its own and summed, the way Medal keeps per-app tracks apart. Removing an app's
  track removes it completely (measured: what was left in "Other apps" sat 90 dB down). Apps
  with their own track are now part of the combined track directly. Older clips keep the
  offset-and-subtract method above.
- The microphone is only ever on its own track; it is no longer mixed into the combined track.
  New clips record which tracks the combined track holds.
- Settings setup (Settings, Video): four questions (what you play, how much game performance
  recording may take, fixed size or frame rate and stretched resolutions, where clips go), then
  a test of a few seconds that encodes moving frames on the graphics card at the matching
  settings, best quality first, and recommends the best one that leaves the asked-for headroom,
  with a bitrate to match. Recording pauses while it runs. Nothing changes until it is applied.
- More video settings: constant quality (CQ) with a peak bitrate, and under "Encoder
  fine-tuning" the tuning mode, multipass, look-ahead, spatial and temporal adaptive
  quantisation with its strength, and the keyframe interval.
- A settings code: every video setting as one short code (`UMBRA-…`) to copy and send, and a
  field to load one. A mistyped code is refused with a reason instead of read wrongly.
- Stretched resolutions: a whole screen is recorded at the monitor's own resolution, so a game
  that switches it to 1440×1080 is recorded as it looks. "Other shapes" chooses between
  stretching such a picture to fill the clip and keeping its shape with black bars; the same
  applies to a custom size and to a resized window.
- The window picker shows each open window as a live preview, games first, with a search field.
- Delete in the player (and the screenshot viewer) deletes the clip and opens the next one;
  Undo in the message brings it back for 5 seconds before it goes to the Recycle Bin.
- The Save clip and Screenshot buttons show their hotkeys inside the button.
- Loading a settings code shows its video settings next to yours in a dialog first. Applying
  them copies your old settings to the clipboard as a code (unless they were the defaults), so
  going back is one paste away.
- Recordings: a Record button in the panel (and a hotkey, Alt+F9) records what is being
  captured to a file until you stop it. Full game sessions (Settings, Recording) record each
  game from the moment it is found until it closes, like Medal's Full Session Recording; a
  second hotkey (Alt+F12) starts or stops the session by hand. The replay buffer keeps running
  meanwhile: the clip hotkey still saves clips, and also drops a bookmark in the recording.
- Bookmarks show on the player's position bar and the editor's timeline; [ and ] jump between
  them, and a click on one in the editor goes there.
- Recordings are written as they record (fragmented MP4), so one cut short by a crash or power
  loss is still playable; Umbra gives it back its .mp4 name on the next start. A change of
  video settings mid-recording closes the file and carries on in a new one.
- The library has a Recordings filter, and recordings carry a Recording or Session badge.
- The panel's top line is now "Replay buffer on/off" with Turn on and Turn off, so it is not
  confused with recording to a file.
- Settings, albums and bookmarks live in Documents\Umbra (wherever Windows keeps Documents),
  so they are easy to find and back up. Thumbnails and other caches moved to
  %LOCALAPPDATA%\Umbra. An existing install's files move over by themselves on the first start.
- A first-start screen: where clips go (and a folder per game or not), when Umbra is ready to
  clip, clip length, the save hotkey, the microphone and starting with Windows, then the
  hotkeys and where Help, the settings setup and the settings file are. Nothing is stored until
  it is finished, so closing Umbra halfway shows it again.
- The About page no longer has a Network line.
- Video presets for kinds of games: "Fast shooters" (120 fps) and "Story and open world"
  (detailed, 60 fps) come with lists of common games, more can be added, and everything else
  uses the default settings. The recorder switches when a game of a preset is found; games in a
  preset count as games even in a window. Clips record which preset they used.
- Sessions can record only the game's window (Settings, Recording) while the replay buffer keeps
  the whole screen: clips of the desktop, a session of just the game.
- A stopped session can be restarted while its game runs: the panel shows Restart session
  (and Record session when full sessions are off).
- Switching to another window no longer ends a game session: a game stays "the game" until its
  program closes.
- Messages over the game when a clip is saved or a recording starts or ends, in a corner of
  your choice. They never take the mouse or keyboard; games in exclusive fullscreen hide them.
- The microphone can also be mixed into the combined track, so every player has your voice;
  it keeps its own track either way. The player now plays the microphone track next to the
  combined one.
- A recording in progress shows first in the library with a live picture, its length and size,
  and a Stop button.
- Recordings open at once: they are still written in fragments while recording (so a crash
  loses nothing), then rewritten as an ordinary MP4 when they stop. A 10-minute recording took
  7.5 s to open before and 18 ms after. Recordings from earlier builds are rewritten on start.
  Hover previews play for recordings again.
- Hovering the player's position bar shows the frame at that point.
- Settings has a page per topic instead of one long scrolling page. While Settings is open its
  pages are listed under it in the sidebar (and inside Settings when the sidebar is folded to
  icons); it reopens on the last page used. Search (Ctrl+F) looks through every page at once and
  counts the matches per page.
- Export threads (Settings, Player and editor): one per core, or a set number to leave room
  for a game.
- Skip the Recycle Bin (Settings, Storage) deletes for good; the delete messages say so.
- Storage limits (Settings, Storage): keep the clip folder under a size, delete clips older
  than a number of days, or both. The oldest go first; favourites, clips in albums (if chosen)
  and anything from the last 10 minutes are kept. Checked after every save, hourly and on
  demand, with a message saying what went.
- Export presets in the editor: Discord (10, 50 or 500 MB: the bitrate, picture size and 60 fps
  cap follow from the clip's length; H.264), YouTube (up to 1440p60 at 40 Mbit/s) and Phone
  (9:16 from the middle, movable, H.264, 60 fps). They write one audio track with the
  microphone mixed in, since sites and Discord play only the first track.
- AMD (AMF) and Intel (Quick Sync) graphics cards can record. Written to FFmpeg's interfaces
  for both but only run on NVIDIA so far; see docs/testing.md.
- Caches moved out of the program folder to %LOCALAPPDATA%\app.umbra.clipper\cache.
- Test under a game's load (Settings, Video): a stand-in game keeps the graphics card busy at
  the screen's size, half the processor's cores busy and a quarter of the memory (up to 8 GB)
  in use. It measures the game's frame rate and 1 % lows alone and while recording with the
  default or a preset's settings, counts dropped recorded frames, and suggests changes (each
  with an Apply button) when recording drops frames or costs the game noticeably.
- The "Fast shooters" preset is now called "FPS" (renamed in existing settings too).
- Fixed: in the overlay's corner choice, Bottom left had no top border.
- Fixed: sessions recording only the game's window ignored a stretched resolution and recorded
  a 1440x1080 game as a 4:3 video, which players show with black bars. A window filling its
  monitor is now recorded at the monitor's own resolution, stretched or fitted as set.
- Fixed: Choose (clip folder, in Settings and on the first-run screen) did nothing. The folder
  picker failed without showing on a thread set up for capture; it now has its own thread and
  opens in front of Umbra.
- Fixed: restarting a session from Umbra's window while the game was minimized recorded the
  whole screen instead of the game's window. The session now records at the size the game is
  played at (the screen it last filled) and picks up the picture when the game is back.
- Pause and resume recordings and sessions: a button next to Stop (in the panel and on the
  live card in the library) and a hotkey (Alt+F8 by default). What happens while paused is left
  out; the file carries on from the next keyframe without a gap, and its time counts only what
  was recorded. The overlay says when it pauses and resumes.
- Never record (Settings, Recording): programs that never count as games, so no session
  records them, even when they fill the screen. Add a program name, a program's full path, or a
  folder for every program inside it.
- Program lists (games, never record, video presets) take a program's full path as well as its
  name, typed or chosen with Choose program; presets apply by path too.
- Editor export reworked: an Export settings button (showing the choice), a line on what the
  file will be (length, picture, frame rate, codec, tracks, about how big) and Export below.
  The settings open in their own window: Original (only your edits), Discord, YouTube, Phone,
  and Custom (codec, picture size, frame rate, and a bitrate or a file size limit); separate
  tracks or one mixed track; the file's name; replace the original; and copy the file when
  it is done, ready to paste into Discord. They are remembered for the next export.
- Copy file in a clip's right-click menu puts it on the clipboard as a file.
- Cutting in the editor: C sets a cut point at the playhead (again on it takes it away), Delete
  takes out the part between cut points under the playhead (it is outlined), Q and W remove
  everything before or after it. Cut points can be dragged. Ctrl+Z and Ctrl+Y undo and redo
  trims and cuts, with buttons next to Cut.
- Magnetic snapping on the timeline: dragging the playhead, a trim handle, a selection or a cut
  point sticks to cut points, bookmarks, trim ends and the edges of parts taken out. A magnet
  button turns it off; Alt drags freely.
- Several keys or buttons per hotkey (for example push-to-talk on both side mouse buttons): a
  + next to each hotkey adds another; any of them works.
- Clip, After the hotkey: keep recording up to 15 s after the clip hotkey before saving, so
  the moment after a play is in the clip. The overlay says so at once.
- Clip, Keep the buffer: on disk instead of in memory. Buffer files go in Umbra's cache folder
  or a chosen one, are deleted on their own (also after a crash), and fall back to memory if
  the disk fails.
- Export formats: MP4, MOV, MKV, WebM (AV1 or VP9 with Opus) and GIF, or audio alone as MP3,
  M4A, WAV, FLAC or Opus. Other formats are saved next to the clip.
- Editor: speed for a part (the selection or the part between cut points): S slow motion,
  F fast, N normal, or right-click the timeline for 0.25x to 4x. Previewed at speed; the pitch
  follows, as in the export. Bands on the timeline show the parts.
- Editor: fades in and out, and through black between clips.
- Editor: a Clips tab (media pool) adds more clips to make one video; a strip under the
  timeline shows them in order to select, drag into another order, take out, or play all.
  Each keeps its own trim, cuts, crop, tracks and speed.
- HDR games: on a screen with Windows' HDR on, Umbra captures in HDR and either converts to SDR
  (BT.2390 roll-off to Windows' SDR brightness, the default) or keeps HDR10 (10-bit BT.2020 PQ
  with HEVC or AV1). The recording panel says which.
- Library sorted by date shows a heading per day (Today, Yesterday, weekdays, then dates). A
  clip's date is when it was recorded: a recording's start (a new tag), else the file's own
  creation time, else when it was written less its length.
- Import (Library): clips from NVIDIA, Medal, AMD ReLive, SteelSeries, Xbox Game Bar, Discord
  and Outplayed folders, or any folder or files, copied into the clip folder by game. MKV, MOV
  and WebM become MP4; the original date is kept for sorting; imported files are left out the
  next time.
- Import can remove the originals after copying: each goes (to the Recycle Bin unless Storage
  skips it) only once its copy checks out (same length and picture size; a rewrapped copy has
  exactly the same number of video and audio packets). Imports run on all processor threads.
- Import progress shows each file being worked on (copying, converting, checking the copy,
  removing the original) with how far it is, the bytes done of the whole import, the speed and
  the time left, and says why big files take a while. With the dialog closed, the progress
  stays at the bottom of the sidebar (a ring when it is collapsed); clicking it opens the
  dialog again. Moving checks only the copy again: the original's packets are counted while it
  is copied, so each file is read twice instead of three times.
- A session ends only when you stop it or its game closes. While it records, no other
  program can become the game, even one that fills the screen or is on your games list. A
  session or recording cut off by the capture starting again (a change of screen mode, the
  game making a new window) carries on in a new file instead of ending, also when sessions
  do not start by themselves. Only the "keep this much free" disk limit still stops one.
- Compressing is about 2.5 times as fast: the graphics card's encoder runs at a quicker
  setting for it (P3 instead of P4), a few percent larger for the same look. A new Frame rate
  choice (Keep it or 60 fps) makes a 144 fps recording smaller still and halves the time
  again. Quality and Frame rate now apply to compressing by hand as well, so they show even
  with compressing old recordings off.
- Stop compressing: the Compressing card in the library has a Stop button, and the message
  that starts compressing has one too. The original stays as it is, nothing half-made is
  left, and the automatic pass leaves that file alone from then on (asking for it by hand
  still works).
- Which games, for the status badge, offers the games running now and the games your clips
  were recorded in, one click each, as well as typing or choosing a program.
- Fixed: Snipping Tool (and other screenshot tools) ended the game's session and started a
  short "Snipping Tool" session, because its overlay fills the screen. Screenshot tools never
  count as games now, and any program that only fills the screen has to do so for 15 seconds
  before it replaces a game that is still running.
- Compress any clip or recording by hand: right-click it, Compress, and pick Smaller,
  Balanced or Better (your setting is marked); right-clicking one of several selected clips,
  or Compress in the selection bar, does all of them. It works whatever their age and
  whether compressing old recordings is on, with the same checks, one after another, and
  waits while a game runs or something is recorded. A message says how much smaller each
  became, or why one stays as it is (already small, HDR). "Shrink" is called "Compress"
  everywhere now.
- Drag clips out of Umbra into Discord, Explorer, a browser upload or any program that takes
  files: press on a clip in the grid or the list and move. A selected clip takes every
  selected one along. It is Windows' own drag and drop, as from Explorer (copy cursor and
  file picture); a click still opens the clip, and Umbra's import overlay stays away while
  its own clips are dragged.
- Settings, Storage, Shrink old recordings (off by default): recordings older than the days
  chosen are encoded again as HEVC at a lower bitrate (Smaller, Balanced or Better; a 1440p
  60 fps recording becomes about 12 Mbit/s at Balanced instead of about 120), in the
  background, on the graphics card, one at a time, never while a game runs, a recording is
  made or an import runs. Every audio track, the tags, the name and the date stay, so albums,
  bookmarks and sorting are unchanged; HDR10 recordings and favourites are left alone, Clips
  too adds saved clips over 200 MB. The original goes (Recycle Bin unless Storage skips it)
  only once the new file is as long, as big a picture and has the same tracks. A shrink shows
  in the library with its progress; Settings shows what it would save and has Shrink now.
- Exports that re-encode (presets, crop, frame-exact cuts, speed changes, fades, other
  formats) decode the video on the graphics card when it can, and a hardware encoder gets
  the frames as they come (NV12) instead of converted. 30 s of a 255 Mbit/s 1440p60 recording
  with a crop: 8.2 s instead of 12.0 s, with 4.2 s of processor time instead of 56.5 s, so
  the PC stays usable meanwhile. Without such a card it decodes in software as before.
- Clips and track audio now load from a small server on this PC instead of the window's
  file protocol: a jump in a 255 Mbit/s recording is ready in about 50 ms instead of about
  1.75 s (test build), so the loading spinner after a jump is rarely seen. It listens on
  127.0.0.1 only, needs a random secret in every address, serves only the clip folder and
  Umbra's caches, and only reads.
- Better game detection. The window in front now counts as a game when it is installed where
  games are: Steam's libraries (from its library list and app manifests), Epic's manifests,
  GOG, Ubisoft and EA installs (from the registry), and Games, Riot Games, Rockstar Games,
  XboxGames folders; windowed games count too. Windows of game engines (Unreal, Unity,
  Source, CryEngine, SDL, GLFW, Godot and others) count as well, and a window filling the
  screen still does. Crash handlers, anti-cheat setups, launchers and store tools (Wallpaper
  Engine, Soundpad, redistributables) never count, and the list of programs that are not
  games is much longer (video players, photo and office apps, editors, store launchers,
  overlays). Splash screens and small dialogs are left out. Nothing touches the game: only
  the window, the registry and files are looked at.
- Games have proper names: the store's ("Broken Arrow"), else the program's own ("World of
  Warships" for WorldOfWarships64.exe), else its folder. Clips get them in their folder and
  file names from now on; clips saved before keep theirs.
- The panel shows the game Umbra sees and why ("installed through Steam", "fills the
  screen"), and the log notes each game found.
- The recording panel on the right is yours to arrange, like a control center: Customize at
  its bottom shows each widget outlined; drag one to move it (or focus it and use the arrow
  keys), remove it with its minus button, and Add widget picks from the panel's own parts
  (replay buffer, save/screenshot/record, capture source, microphone, usage) and about thirty
  settings: switches become tiles that light up when on, two to a row; choices, numbers and
  sliders take the full width. A setting changed there is saved at once. Nothing about the
  layout changes until Save; Cancel leaves it, Reset goes back to how Umbra came. The layout
  is kept in the settings file.
- Status over the game (Settings, Clips): a small badge that stays on screen with REC and the
  time while recording, Paused, Replay and how much the buffer holds, or Off. Off, Always, or
  In games (chosen programs by name, path or folder, or any detected game), in a corner of its
  own on the game's screen. It never takes the mouse or keyboard.
- The badge and the messages over the game are kept out of screen captures, so they are never
  in a clip or recording (before, a "Clip saved" message could end up in the next clip).
- A clip being saved shows at the top of the library from the hotkey on: the seconds still
  recorded after it, then writing, until the clip is there. A clip that cannot be saved says
  why.
- Back from watching a clip, the library is where it was instead of at the top.
- Import by drag and drop: videos or folders dragged onto the window from Explorer show
  "Drop to import" over the whole window, and dropping opens the import with them ticked and
  looked through (games guessed, clips already there left out), ready to copy or move.
- Fixed: recordings and clips could show one still picture of the screen until Umbra was
  restarted. Windows ends a screen capture when the display setup changes (a game switching
  resolution or to exclusive full screen, HDR on or off, a screen asleep or plugged in again,
  a driver reset) and Umbra kept repeating the last picture. Now the capture is started again
  when Windows ends it or no frame came for 2 s (waiting longer, up to 30 s, while a screen
  really is still), keeping the replay buffer; a driver reset restarts the recorder.
- Umbra keeps a log file, `logs\umbra.log` in its local app data folder
  (`%LOCALAPPDATA%\app.umbra.clipper`), so a problem can be looked into afterwards.
- Record asks what to record: a display or an open app, with live previews; what the replay
  buffer captures is picked to begin with. Something else gets a recorder of its own, so the
  buffer and saving clips carry on (and the buffer is not started for it when it is off). The
  record hotkey still records the capture straight away.
- Full screen in the player: the button, F, or a double-click on the picture. The controls
  float over the bottom and hide with the cursor after 2.5 s while playing; Esc leaves full
  screen first, then the player.
- Fixed: opening a recording finished in place froze the whole app for a long time. Its
  fragment headers had become thousands of small boxes before the index, and the player read
  them one by one, each a request through the window's main thread. Now everything before the
  index is one data box (three boxes in all, like a copied file): a 10 GB recording opens in
  11 ms instead of 2 s in the test build, and longer ones were far worse. Recordings already
  finished that way are rewrapped at the next start, in a moment each.
- Fixed: jumping while playing (player and editor) stuttered: short stops and repeated sound
  after the jump. Now the picture and every audio track stop, seek, and start together once
  about 1.5 s is loaded at the new spot (less near the end); a spinner shows if that takes
  more than a moment. Running out of data while playing waits for a little more before
  going on, instead of stopping again a moment later.
- Recordings are finished in place: instead of copying the whole file into an ordinary MP4
  after it stops (minutes, and twice the disk space, for a long session), the small fragment
  headers are read and a full index is written at the end. About a second for any length,
  nothing extra on the disk. A journal puts the file back if Umbra stops part-way, and a file
  that does not look as expected is still copied the old way.
- Export: cuts are fast by default. The start and each cut move to the nearest keyframe and
  the video is copied, not encoded again, so a trimmed or cut 90-minute session exports at
  disk speed instead of re-encoding everything. Export settings, Cuts: Frame-exact keeps the
  old behaviour.
- A running export shows at the bottom of the sidebar like an import, with the percent and
  time left on any screen; clicking it goes back to the editor.
- A recording that is being finished (rewritten as an ordinary MP4 after it stops, which takes
  minutes for a long session) shows in the library as a placeholder card at the top: a
  skeleton thumbnail, "Finishing", the percent, bytes read of the total and the time left.
  It turns into the clip when done. Recordings left in fragments that Umbra finishes at
  start show the same way, and are not listed twice.
- Fixed: Start with Windows did nothing when Windows' startup entry was missing (it was only
  written when the setting was switched, so an entry that was removed, or a setting saved some
  other way, stayed broken). Umbra now puts the entry right at every launch, pointing at
  itself. Debug builds and test copies with their own settings folder no longer touch it.
- Import checks for room first: a file is left where it is, with the reason, when the clip
  drive would drop under Storage's Keep free (files being imported at the same time count).
  The finished message names the first problem.
- Storage, Keep free on the drive (5 GB): under it a recording or session stops and is saved,
  none starts, and a Windows notification, the overlay and the window say so.
- Fixed: jumping in a clip with a separate microphone track (player) or in the editor repeated
  a bit of sound while the video caught up.
- Fixed: when the screen went back to a larger mode (a stretched game closing, Battlefield V
  at 2304×1440), the first frame came in the capture's old, smaller buffer; its copy into the
  encoder was skipped by Direct3D and an older picture was recorded instead. Such a frame is
  now left out and the last one shown until a whole one arrives.
- Fixed: a capture restart that failed while the screen changed mode doubled the wait before
  the next try, so the clip showed a still picture for 4 s or more; it is tried again after
  0.5 s now. A capture Windows ended whose restart failed was tried again on every frame.
- Fixed: a microphone (or app) that cannot be opened is logged once, not every 3 s; a missing
  microphone filled the log file within a day. NVIDIA Control Panel, `electron.exe` and WoWs
  Monitor no longer count as games when they fill the screen.
- Fixed: a microphone that is not connected was recorded as silence without a word, and the
  panel still said Live while the push-to-talk key was held. The panel now says Not connected
  with the reason, and other silent tracks are listed under the replay buffer.
- Fixed: a game with a preset of other video settings (the FPS preset's 120 fps), or the screen
  turning HDR on or off, emptied the replay buffer, so a clip saved right after held only a
  few seconds. The old recorder now keeps running until the new one holds a full buffer, and
  clips come from whichever holds more. Changing the settings by hand still starts afresh.
- Saving a clip no longer waits for the recorder to start (opening each audio source can take
  seconds), and two clips saved in the same second (the hotkey and the tray) no longer pick the
  same file name.
- Fixed: shrinking a recording sent the original to the Recycle Bin (or deleted it) before the
  smaller file was in place; if that move failed, the next pass deleted the smaller file as a
  leftover. A checked smaller file whose original is gone is now put in its place.
- Screenshots and previews of a screen in HDR are converted like recordings instead of looking
  washed out. Stills share one graphics device instead of making one each (every 5 s for each
  preview in the panel).
- The log file starts over past 5 MB while Umbra runs, not only at launch.
- The controller no longer lists every screen (or window and process) ten times a second; the
  capture is looked up every 2 s and a captured window's program every second.

## 0.1.0

First build of everything in the plan. It runs on the development PC.

- Screen or single-window capture with hardware encoding (NVIDIA: H.264, HEVC, AV1), custom
  resolution, frame rate and bitrate or constant quality.
- Replay buffer in RAM; a clip is written on a hotkey without re-encoding.
- One audio track per source (game, other programs, everything else, microphone) plus a combined
  first track.
- Recording always, only while a game runs, or by hand. Launch at startup.
- Clip library with thumbnails, an editor with trim and crop, and export that copies the video
  when it can.
- Tray icons for recording and microphone state, no network code, no accounts.
