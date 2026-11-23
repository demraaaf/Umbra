# Umbra feature inventory

Audit of the working tree on 2026-11-18, `main` at `e448423`, including the uncommitted auto-clip work. Updated the same day for auto-clip (committed as `0170706`), the keyboard and mouse overlay, software cameras and burn-in from the library.
Based on the code; README and CHANGELOG claims were checked against it.

Path shorthands: `core/` = `app/crates/umbra-core/src/`, `app/` = `app/crates/umbra-app/src/`,
`cli/` = `app/crates/umbra-cli/src/`, `ui/` = `app/ui/src/lib/`.

---

## 1. Overview

Umbra is an offline game clipper for Windows. It runs a GPU-encoded replay buffer, records sessions,
and comes with a library, a player and a multi-clip editor. It's a Rust 2024 workspace with three
crates: `umbra-core` is the engine, `umbra-app` is the Tauri 2 shell and `umbra-cli` is a headless
harness. The UI is a Svelte 5, TypeScript, Tailwind 4 and shadcn-svelte (bits-ui) SPA that runs in
WebView2. Media work goes through FFmpeg 8.1 (`ffmpeg-next` 9, an LGPL shared build with 5 DLLs). The
OS is reached through the `windows` 0.62 crate (Win32 and WinRT). The core and the UI talk over Tauri
IPC: 66 `#[tauri::command]`s, which are mapped 1:1 by `COMMANDS` in `ui/api/types.ts`, plus about 19
events pushed from the controller's event sink (`app/main.rs`). Video and track audio reach the webview
through a 127.0.0.1 HTTP range server that requires a secret token (`app/media.rs`). Thumbnails use the
asset protocol. A mock backend (`ui/api/mock.ts`) lets you work on the UI in a browser.

**OS requirements implied by the code.** Nothing gates on the OS version.

| Requirement | Why | Where |
| --- | --- | --- |
| Windows 10 2004 (19041)+ | WASAPI process-loopback (per-app audio), `WDA_EXCLUDEFROMCAPTURE` for overlays, WGC cursor toggle | `core/audio.rs:444`, `app/overlay.rs:84`, `core/capture.rs:335` |
| **Windows 11 24H2 (26100)+** for >60 fps | `GraphicsCaptureSession.SetMinUpdateInterval`. Older builds cap WGC at 60 fps (a warning is logged) and the encoder repeats frames | `core/capture.rs:339` |
| Windows 11 / build 20348+ | `SetIsBorderRequired(false)`. Errors are ignored, so older builds show the yellow capture border | `core/capture.rs:334` |
| D3D11 FL 11.0, `IDXGIFactory6`, `IDXGIOutput6` | Device on the high-performance adapter, HDR detection | `core/d3d.rs`, `core/hdr.rs` |
| NVIDIA, AMD or Intel GPU with a HW encoder | No software encoder for recording | `core/encode.rs:371` |
| WebView2 runtime, x64 | Tauri 2. `fetch-deps.ps1` only fetches win64 FFmpeg | `app/tauri.conf.json`, `scripts/fetch-deps.ps1` |

---

## 2. Implemented features

### Capture
| Feature | Status | Where | Notes |
| --- | --- | --- | --- |
| Screen capture, GPU-resident frames | Complete | `core/capture.rs`, `core/d3d.rs` | Screens use DXGI Desktop Duplication (sees exclusive-fullscreen games; cursor drawn with GDI, none on HDR screens) and fall back to WGC when the screen is on another GPU. Windows use WGC |
| Monitor capture | Complete | `core/capture.rs`, `core/controller.rs` `resolve_target` | Primary by default. Looked up again by device name after display changes |
| Window capture (exe and title) | Complete | `core/wnd.rs`, `core/controller.rs` | Holds the last frame while minimized. Follows resizes |
| Resolution | Complete | `core/scale.rs` (D3D11 video processor), `core/settings.rs` | Native monitor res by default, or custom W×H (even, ≤16384). Stretch or Fit for other aspect ratios (stretched 4:3) |
| FPS 1–360 | Complete | `core/timer.rs`, `core/recorder.rs` | High-res waitable timer. Repeats the last frame when nothing new arrives. >60 fps needs 24H2 (see §1) |
| Cursor capture toggle | Complete | `VideoPrefs.capture_cursor` | |
| HDR | Complete | `core/hdr.rs` | Detects an HDR output and captures FP16 scRGB. Converts to SDR (BT.2390 roll-off to Windows' SDR white) or keeps HDR10 (P010, BT.2020/PQ, HEVC/AV1 only). Falls back to SDR |
| Capture recovery | Complete | `core/recorder.rs` `run()` | Restarts on WGC `Closed` or a 2 s stall (backoff up to 30 s). A device-removed error rebuilds the recorder |
| Screenshots (PNG/JPEG, own hotkey) | Complete | `core/snapshot.rs` | One-shot WGC session, HDR converted |
| Live source previews | Complete | `core/snapshot.rs`, `app/commands.rs` `list_capture_sources` | |
| Webcam as a separate stream | Complete | `core/camera.rs`, `core/dshow.rs` | Media Foundation (MJPEG/YUY2 → ARGB32), then HW-encoded with the recording's codec. Cameras only DirectShow lists (OBS Virtual Camera, other software cameras) go through the Sample Grabber (NV12, I420, YV12, YUY2, RGB24/32, ARGB32 → BGRA by swscale); one both list is Media Foundation's. A camera that stops sending leaves a gap and is retried every 2 s. Alpha from a camera is detected and logged, not kept (the encode is 4:2:0). Hotkey Alt+F7. Burned in only on export |
| Keyboard and mouse input (overlay) | Complete | `core/input.rs`, `core/inputrules.rs`, `core/inputguard.rs`, `core/inputfile.rs` | Off by default. Raw Input sink on its own thread (no hooks, no polling), QPC-stamped. Per-program rules (first match; detected game, programs, or any) keep keys in full, redacted (text keys anonymous) or not at all, and the mouse in full, buttons only or not at all; the stricter rule within 50 ms of a switch. UI Automation password guard (fails closed outside full rules), chat guard per game, nothing while Umbra is in front. Decided 300 ms late, before anything is written. Kept in the replay buffer (spills to disk like the rest) and in a sidecar `<clip>.input` (versioned blocks with CRC; crash-safe for recordings) cut to the clip's real keyframe. Follows rename, delete, cleanup and in-place saves. Toggle hotkey (unbound by default), panel widget |
| Logo/watermark | Complete | `core/settings.rs` `LogoPrefs`, `core/burn.rs` | Placement is stored per clip. Burned in on export |

### Encoding
| Feature | Status | Where | Notes |
| --- | --- | --- | --- |
| NVENC H.264 / HEVC / AV1 | Complete | `core/encode.rs` | D3D11 frames go straight to FFmpeg as BGRA (P010 for HDR10). No B-frames |
| AMD AMF, Intel Quick Sync | Partial | `core/encode.rs` | Option mapping is written but untested (CHANGELOG says so). QSV goes through a derived device with a fixed 24-frame pool |
| Software encoder for recording | Not implemented | — | Any other vendor fails with "no hardware encoder" |
| Rate control | Complete | `core/encode.rs` | CBR, VBR (target and peak), CQ (with peak), CQP |
| Fine-tuning | Complete on NVENC, mapped approximately on AMF/QSV | `core/encode.rs` | Preset P1–P7, tune hq/ll/ull, multipass, look-ahead, spatial/temporal AQ and strength, keyframe interval 1–10 s |
| Audio codec | Complete | `core/aac.rs` | FFmpeg native AAC, 32–512 kbit/s (default 160) |
| Container | Complete | `core/mux.rs`, `core/defrag.rs` | MP4: faststart for clips. Recordings are written fragmented and finished in place, with a crash journal |
| Per-game video presets | Complete | `core/settings.rs` `default_presets` | "FPS" (120 fps) and "Story" (60 fps) by exe, path or folder. Switching keeps the old buffer until the new one fills |
| Settings share code (`UMBRA-…`) | Complete | `core/share.rs` | Crockford base32 with a check byte |
| Setup wizard / encoder benchmark | Complete | `core/benchmark.rs`, `ui/components/app/settings/BenchmarkDialog.svelte` | |
| Load test with a stand-in game | Complete | `core/gameload.rs`, `core/loadtest.rs` | Suggestions come with Apply buttons |
| Cost estimate | Complete | `core/estimate.rs` | Encoder figures are calibrated on one GPU (RTX 5070 Ti) |

### Audio
| Feature | Status | Where | Notes |
| --- | --- | --- | --- |
| Per-app capture (WASAPI process loopback) | Complete | `core/audio.rs` | Captures the whole process tree, re-attaches when an app restarts, and records a missing source as silence with a reason |
| Track sets | Complete | `core/controller.rs` `audio_config`, `core/others.rs` | **Split**: Game (follows the detected game's PID), one track per listed app (default `Discord.exe`), "Other apps" (every other audio-session process captured on its own and summed), Microphone. **System**: one track with everything except Umbra |
| Combined first track ("Mix") | Complete | `core/recorder.rs` `Mixer` | Added when ≥2 tracks are in the mix. The mic is opt-in. Hard clip at ±1.0. The `umbra_mix` tag lists its parts |
| Storage in the file | Complete | `core/mux.rs` | One AAC stream per track, named after the source. The camera stream comes last |
| Format | Complete | `core/align.rs`, `core/clock.rs` | 48 kHz stereo f32. WASAPI auto-converts. QPC-timestamped and aligned to 1024-sample chunks |
| Mic device, push-to-talk | Complete | `core/hotkey.rs` `KeyHoldWatcher`, `core/recorder.rs` | 5 ms fade gate. Multiple PTT keys |
| Mic processing (noise suppression, gain, AGC) | Not implemented | — | The mic is recorded raw |
| Per-track volume at export (0–200%) | Complete | `core/export.rs`, `core/unmix.rs` | The mix is rebuilt from its parts. Older clips: subtracts the app from "Other apps" (about 25 dB quieter) |
| Waveforms | Complete | `core/waveform.rs` | |

### Replay buffer and recording
| Feature | Status | Where | Notes |
| --- | --- | --- | --- |
| RAM ring buffer, GOP-aligned | Complete | `core/buffer.rs` | Length 5–600 s, plus 0–15 s recorded after the hotkey |
| Buffer on disk (optional) | Complete | `core/spill.rs` | Delete-on-close temp segments of 256 MB. Falls back to RAM on disk errors |
| Clip save | Complete | `core/recorder.rs` `save_clip` | Packet copy (no re-encode) to `.part`, then rename. Starts on a keyframe |
| Recording to file | Complete | `core/controller.rs` `drive_file` | Manual (Alt+F9), full game sessions (auto or Alt+F12), pause/resume (Alt+F8). Clip hotkey adds bookmarks. Can record another source or the game window with its own recorder. Carries on in a new file after interruptions |
| Crash safety | Complete | `core/recorder.rs` `recover_recordings`, `core/defrag.rs` | Fragmented MP4 while recording, renamed back on start. Journal-based in-place finalize |
| Low-disk guard | Complete | `core/controller.rs` `guard_disk` | Default 5 GB free. Stops or blocks recordings and notifies |

### Triggers
| Feature | Status | Where | Notes |
| --- | --- | --- | --- |
| Global hotkeys | Complete | `core/hotkey.rs` | Polls `GetAsyncKeyState` every 4 ms. No hooks. Combos plus Mouse3/4/5, several bindings per action, rebound live. Keys aren't swallowed |
| Default bindings | — | `core/settings.rs` | Clip Alt+F10, screenshot Alt+F11, record Alt+F9, session Alt+F12, pause Alt+F8, camera Alt+F7, PTT V |
| Auto-clip: CS2 Game State Integration | Complete (CS2 only) | `core/autoclip.rs`, `core/cs2.rs`, `ui/components/app/settings/AutoClipSettings.svelte` | Localhost listener with a token. Kills counted per round (every kill, 2K, 3K, 4K, ace); headshot, knife, Zeus, collateral and blind kills named in the clip and able to clip alone; deaths, round wins; game-mode filter. Per-event pre/post seconds, merge window for roundless modes, own sound, event log, one-click config install |
| Tray / UI save | Complete | `app/main.rs` `on_tray_menu` | |

### Clip management
| Feature | Status | Where | Notes |
| --- | --- | --- | --- |
| Library scan | Complete | `core/library.rs` `scan`, `app/clips.rs` | MP4 and images, probed in parallel. Sorted by recorded date with day headings. No metadata cache |
| Thumbnails, hover play, scrub preview | Complete | `core/library.rs`, `ui/components/app/ScrubPreview.svelte` | Cached in `%LOCALAPPDATA%\app.umbra.clipper\cache` |
| Naming | Complete | `core/naming.rs`, `core/gamedb.rs` | Folder and name templates (`{game}`, `{date}`, `{time}`). Store or version-info game names. Auto-clips are named after the event |
| Rename, delete, reveal | Complete | `app/clips.rs` | Recycle Bin or permanent delete, 5 s undo. Reveal through `explorer.exe /select` |
| Albums and Favorites | Complete | `core/albums.rs` | An album can hide its clips from the main list |
| Tags | Not implemented | — | Albums and favorites only |
| Search, filters, sort, grid/list, multi-select | Complete | `ui/components/app/clips/*` | Middle-click sweep selects. Layer filter (Has camera, Has input) |
| Layer badges | Complete | `core/burnin.rs` `layers`, `ui/components/app/clips/LayerBadges.svelte` | Camera, input and logo icons on cards and rows, read from the file (camera stream, `.input` sidecar, `umbra_logo` tag whose picture exists), so imported and older clips count |
| Properties | Complete | `ui/components/app/ClipProperties.svelte` | Read from `umbra_*` tags |
| Storage limits | Complete | `core/cleanup.rs` | Size cap and/or age. Keeps favorites, albums and the last 10 min. Runs hourly and after each save |
| Compress ("shrink") | Complete | `app/shrink.rs` | Background or manual HEVC re-encode. Checked before the original is removed |
| Burn in | Complete | `core/burnin.rs`, `app/burnin.rs` | Right-click or selection bar: Camera, Input overlay, Logo (those the clips have) or All layers. A queue on its own thread, progress card, Stop. The clip's own placements (its tags), else Settings'. Source codec at the source's bitrate (the "Original" preset), audio copied, unburned layers kept (camera stream, tags, sidecar copied). `<name> (burned).mp4`, or in the original's place when set, after checking length, tracks, size and camera. Starts at once, like an editor export (does not wait for a game to close as Compress does) |
| Import | Complete | `app/import.rs` | From NVIDIA, Medal, AMD, SteelSeries, Xbox, Discord, Outplayed or any folder or file. Copy or move with verification. Drag-and-drop into the window |

### Editing and export
| Feature | Status | Where | Notes |
| --- | --- | --- | --- |
| Trim and cuts | Complete | `core/export.rs`, `ui/editor/session.svelte.ts` | Keyframe-snapped (copy) by default or frame-exact. Undo/redo, magnetic snapping |
| Crop | Complete | `ui/editor/crop.ts` | Aspect presets: Free, 16:9, 4:3, 1:1, 9:16 |
| Per-track keep, mute, volume; mix-only output | Complete | `core/export.rs` | |
| Copy vs re-encode | Complete | `core/export.rs` | D3D11VA decode, then HW encode. Software fallbacks: libopenh264, libkvazaar, libsvtav1, libvpx-vp9 |
| Speed and fades | Complete | `core/render.rs` | 0.25–4× (pitch follows). Fade in, fade out and through black |
| Multi-clip sequence | Complete | `ui/components/app/editor/MediaPool.svelte`, `core/render.rs` | |
| Output formats | Complete | `core/render.rs` | Video: MP4, MOV, MKV, WebM, GIF. Audio only: MP3, M4A, WAV, FLAC, Opus |
| Export presets | Complete | `ui/editor/exportPresets.ts`, `app/clips.rs` `shape_export` | Original. Discord (10/50/500 MB size target). YouTube (≤1440p60, 40 Mbit/s). Phone (9:16). Custom (codec, size, fps, bitrate or size limit) |
| Camera and logo burn-in | Complete | `core/burn.rs` | |
| Input overlay | Complete | `core/inputdraw.rs`, `ui/input/overlay.ts`, `ui/input/layouts.json` | WASD cluster, compact keyboard, mouse (buttons, wheel, movement) and a keys-pressed ticker, from one layout file for both renderers. Live canvas in the player and editor at the clip's own time, so trims, cuts, speed and sequences follow. Export draws it with Direct2D/DirectWrite, only when what shows changes, blended like the camera. Per clip: on or off and placement in the editor |
| Player | Complete | `ui/screens/PlayerScreen.svelte` | Frame step, seek step, fullscreen, up to 360 fps, bookmarks |

### Sharing
| Feature | Status | Where | Notes |
| --- | --- | --- | --- |
| Upload targets | Not implemented | — | Deliberate: no network code |
| Copy file to clipboard (CF_HDROP) | Complete | `app/clipboard.rs` | Pastes into Discord or Explorer. Optional after export |
| Drag clips out (`SHDoDragDrop`) | Complete | `app/dragout.rs` | |
| Discord-specific | Partial | `ui/editor/exportPresets.ts`, `core/settings.rs` | Size-target preset and a Discord track by default. No Rich Presence or webhooks |

### Game detection
| Feature | Status | Where | Notes |
| --- | --- | --- | --- |
| Detection | Complete | `core/games.rs`, `core/gamedb.rs` | User list and preset lists. Store installs (Steam, Epic, GOG, Ubisoft, EA, Games folders), engine window classes, fills-the-screen. Deny lists and a "never record" list. 15 s takeover rule |
| Per-game settings | Complete | `core/settings.rs` `video_for` | Video presets matched by exe, path or folder |
| Auto-start | Complete | `core/controller.rs` `tick` | Modes Always, Games or Manual. Optional full sessions per game |

### UI/UX
| Feature | Status | Where | Notes |
| --- | --- | --- | --- |
| Settings pages | Complete | `ui/screens/SettingsScreen.svelte`, `ui/stores/settingsNav.svelte.ts` | Capture, Video, Audio, Camera and logo, Clip, Auto-clip, Screenshots, Recording, Player and editor, Storage, Help, About. Search with Ctrl+F |
| First-run onboarding | Complete | `ui/screens/OnboardingScreen.svelte` | |
| Customizable recording panel (widgets) | Complete | `ui/panel/widgets.ts` | |
| In-game overlay | Complete | `app/overlay.rs`, `Overlay.svelte` and `StatusBadge.svelte` (UI `src/` root) | Toasts and a status badge. Click-through and excluded from capture. Exclusive fullscreen hides them |
| Windows notifications | Complete | `app/overlay.rs` | Low disk only |
| Tray | Partial | `app/tray.rs` | Two code-drawn icons (state, mic live). Menu: Open, Save, Start recording, Camera, Quit. The toggle label never changes |
| Start with Windows, start minimized | Complete | `core/startup.rs` | HKCU Run with `--minimized`. Fixed up on every launch |
| Close → tray, single instance | Complete | `app/main.rs` | Named mutex. A second launch exits without focusing the first |
| Themes | Dark only | UI `index.html` (`class="dark"`) | `mode-watcher` is installed but unused |

### Infrastructure
| Feature | Status | Where | Notes |
| --- | --- | --- | --- |
| Config | Complete | `core/settings.rs` | `Documents\Umbra\settings.json` (plus `albums.json`, `recordings.json`). Atomic write, normalization, migration from `%APPDATA%`. `UMBRA_SETTINGS_DIR` / `UMBRA_CACHE_DIR` overrides |
| Logging | Complete | `app/main.rs` `Logger` | `%LOCALAPPDATA%\app.umbra.clipper\logs\umbra.log`, Info level, rotates at 5 MB |
| Crash handling | Partial | — | Data is crash-safe (journal, delete-on-close buffers, `.part` files). No panic hook, no minidumps |
| Auto-update | Not implemented | `scripts/check-no-network.ps1` | Deliberate: the updater plugin is on the deny list |
| Installer | Complete | `app/tauri.conf.json`, `scripts/build.ps1` | NSIS per-user, FFmpeg DLLs bundled. Not code-signed |
| Network-isolation guard | Complete | `scripts/check-no-network.ps1` | Crate deny list, CSP, capabilities, socket whitelist |
| Tests | Partial | `core/*` | 156 Rust `#[test]`s, 7 `#[ignore]` (need a GPU or hardware). Export tests shell out to `ffmpeg.exe`. No UI tests (only `svelte-check`) |
| CI | Not implemented | — | No workflow files in the repo |
| CLI harness | Complete | `cli/main.rs` | `monitors`, `cameras`, `spike`, `colortest`, `record`, `daemon`, `clips`, `thumb`, `export` |

---

## 3. Partial / unfinished work

- **Auto-clip has one detector (CS2).** The audio and screen-region detectors are designed, not built.
- **The input overlay's labels are US layout.** Keys match by scan code, so the positions are right on any layout, but an AZERTY user sees `W` where they press `Z` (`core/inputdraw.rs`).
- **`EventRule.min_confidence` has no effect in practice.** It's in the settings and checked in `core/autoclip.rs:198`, but no UI control exists and the only detector always sends `1.0` (`core/cs2.rs:124`).
- **AMF and QSV are untested** (CHANGELOG). QSV gets BGRA surfaces (`core/encode.rs:434`). Whether `h264_qsv` accepts them is unverified.
- **No software encoder for recording**, though export already has software fallbacks.
- **Tray toggle label** stays "Start recording" (`app/tray.rs:57`).
- **Deliberate shortcuts marked with `ponytail:`** (no TODO or FIXME exists anywhere):
  - `core/controller.rs:249`: two recorders run after a preset or HDR switch.
  - `core/recorder.rs:1258`: trusts the resized pool to send whole frames.
  - `core/autoclip.rs:111`: tag offsets are measured from the requested start, not the keyframe.
  - `core/burn.rs:474`: reads the file twice.
- **Ignored tests** need a GPU or hardware:
  - `core/encode.rs:715`
  - `core/benchmark.rs:655`
  - `core/gamedb.rs:750`
  - `core/gameload.rs:452`
  - `core/scale.rs:308`
  - `core/snapshot.rs:229`
  - `core/unmix.rs:287`
- **Dangling doc references.** `core/estimate.rs` cites `docs/spike-results.md` and the CHANGELOG cites `docs/testing.md`, but `/docs/` is gitignored.
- **Unused UI dependencies:** `mode-watcher` and `@internationalized/date` (`app/ui/package.json`).
- **Wiring is complete.** All 66 commands are called from the UI (`hide_overlay` from `Overlay.svelte`). Every Rust settings field exists in `types.ts`, and only `minConfidence` has no control.

**README vs code**

| README claim | What the code does |
| --- | --- |
| "nothing is written to disk until you save a clip" | Not true with *Clip → Keep the buffer on disk* (`core/spill.rs`, off by default) |
| "for Windows 11" | Most features need only Windows 10 2004+. Capture above 60 fps needs 11 24H2 (§1). Nothing checks the OS version |
| "check-no-network.ps1 … meant to run in CI" | There is no CI config in the repo |
| "The app never talks to the internet" | True for the app. `tauri.conf.json` sets no `webviewInstallMode`, so I believe Tauri's NSIS default (download the WebView2 bootstrapper when WebView2 is missing) applies to the installer |
| Status table | Out of date. It doesn't mention camera and logo, auto-clip, albums, import, compression, sessions, HDR or export formats |

---

## 4. Platform-specific code

There's no abstraction layer: no traits and no `cfg(windows)` gates. Every API below is called directly
from the `windows` crate, FFmpeg's Windows-only hwcontexts, or Tauri/rfd. Porting would touch nearly
every core module.

| API | Used for | Where | Abstracted? |
| --- | --- | --- | --- |
| Windows.Graphics.Capture (WinRT) + `IGraphicsCaptureItemInterop` | Screen and window capture, stills | `core/capture.rs`, `core/snapshot.rs` | No |
| Direct3D 11 / DXGI (`IDXGIFactory6`, `ID3D11VideoProcessor`, `D3DCompile`) | Device, scaling, HDR shaders, load test, benchmark | `core/d3d.rs`, `scale.rs`, `hdr.rs`, `gameload.rs`, `benchmark.rs` | No |
| FFmpeg D3D11VA / QSV hwcontexts | Zero-copy encode, HW decode on export | `core/encode.rs`, `core/export.rs` | No |
| GDI monitors + DisplayConfig (`QueryDisplayConfig`, SDR white level) | Monitor list, native mode, HDR white | `core/capture.rs`, `core/hdr.rs` | No |
| WASAPI (process loopback, `IMMDeviceEnumerator`, `IAudioSessionManager2`) | Audio capture, device and session lists | `core/audio.rs`, `core/devices.rs`, `core/others.rs` | No |
| Media Foundation (`MFCreateDeviceSource`, `IMFSourceReader`) | Webcam | `core/camera.rs` | No |
| DirectShow (`ICreateDevEnum`, `ICaptureGraphBuilder2`, qedit's Sample Grabber) | Software cameras | `core/dshow.rs` | No |
| `PlaySoundW` | Save chime | `core/sound.rs` | No |
| `GetAsyncKeyState`, `SendInput` | Hotkeys and PTT. `SendInput` is the CLI's `--press-after` | `core/hotkey.rs` | No |
| ToolHelp32, `OpenProcess`, `QueryFullProcessImageNameW` | Processes, game PIDs | `core/process.rs`, `core/games.rs`, `core/apps.rs` | No |
| `EnumWindows`, `GetForegroundWindow`, `GetClassNameW`, `DwmGetWindowAttribute` | Window list, game detection | `core/wnd.rs`, `core/games.rs` | No |
| Registry (`RegOpenKeyExW`, `RegSetValueExW`) | Startup entry, GOG/Ubisoft/EA installs | `core/startup.rs`, `core/gamedb.rs` | No |
| `SHGetKnownFolderPath` | Documents folder | `core/settings.rs` | No |
| QPC, `CreateWaitableTimerExW` (high-res), `SetThreadPriority` | Clock, frame pacing | `core/clock.rs`, `core/timer.rs`, `core/recorder.rs`, `core/audio.rs` | No |
| PDH (`\GPU Engine(*)`), `GetProcessMemoryInfo`, `GlobalMemoryStatusEx`, `GetDiskFreeSpaceExW` | Usage meters, estimates, disk guard | `core/usage.rs`, `core/estimate.rs`, `core/disk.rs` | No |
| `FILE_FLAG_DELETE_ON_CLOSE` (`OpenOptionsExt`) | Disk buffer | `core/spill.rs` | No |
| `SetProcessDpiAwarenessContext`, `SetConsoleCtrlHandler`, `GetLocalTime` | DPI, CLI Ctrl+C, timestamps | `core/util.rs` | No |
| `CreateMutexW` | Single instance | `app/main.rs` | No |
| `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` | Overlays kept out of captures | `app/overlay.rs` | No |
| Raw Input (`RegisterRawInputDevices` sink), `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` | Keyboard and mouse for the overlay, the program in front | `core/input.rs` | No |
| UI Automation (`IUIAutomation` focus events, `IsPassword`) | Password guard | `core/inputguard.rs` | No |
| Direct2D, DirectWrite, WIC | Drawing the input overlay for export | `core/inputdraw.rs` | No |
| OLE clipboard (CF_HDROP), `SHDoDragDrop`, `SHFileOperationW`, `explorer.exe` | Copy, drag-out, Recycle Bin, reveal | `app/clipboard.rs`, `app/dragout.rs`, `app/clips.rs` | No |
| WebView2, `rfd` (IFileDialog), `tauri-plugin-notification` | UI, pickers, toasts | `app/*` | Through Tauri/rfd only |

---

## 5. Known issues and risks

1. **A broken JSON file loses its data on the next save.** An invalid `settings.json` loads as defaults and is overwritten on the next save. The next save can be the camera hotkey or a CS2 config install, not only a settings change. `albums.json` and `recordings.json` behave the same way. No backup is kept. (`core/settings.rs:1183`, `core/albums.rs`, `core/recordings.rs`)
2. **Startup failures are silent in release builds.** The binary has no console and no panic hook. A `Controller::start` failure is only `eprintln!`'d and the app exits (`app/main.rs:240`). Panics in worker threads never reach `umbra.log`.
3. **GPU support is narrow.** Recording needs an NVIDIA, AMD or Intel adapter, so VMs, Basic Render and other vendors fail. The AMF and QSV paths are unverified.
4. **Capture above 60 fps only works on 11 24H2+.** On older builds the 120 fps "FPS" preset records duplicated frames and only logs a warning (`core/capture.rs:342`).
5. **The "detected" clip tag is wrong for most games.** It ignores `GameInfo.reason`, so Steam- or engine-detected games show "fullscreen program" and preset games aren't marked "listed" (`core/controller.rs:1529`).
6. **The library doesn't scale well.** `list_clips` runs an FFmpeg probe on every file on every call, with no metadata cache (`core/library.rs:466`).
7. **The controller does more work than needed while idle.** Every 100 ms tick takes a ToolHelp process snapshot whenever any game list is non-empty, and the default presets make that always true (`core/games.rs:316`). Separately, 7 threads poll the keyboard every 4 ms.
8. **The media server:**
   - Spawns one unbounded thread per connection (`app/media.rs:56`).
   - Allows the dev origin `http://localhost:1420` in release (`app/media.rs:28`).
   - Mitigating factors: the 127.0.0.1 bind and the token. `random_token` is duplicated in `core/settings.rs` and `app/media.rs`.
9. **A preset or HDR switch costs double for a while.** Encoder load and buffer memory double for one buffer length (`core/controller.rs:249`).
10. **The mix track can distort.** It's summed and hard-clipped with no limiter, so a loud game plus voice chat clips (`core/recorder.rs:179`).
11. **Single-instance launch is confusing.** Starting Umbra again while it sits in the tray does nothing visible (`app/main.rs:158`).
12. **Distribution:**
    - The installer is unsigned, so expect SmartScreen warnings.
    - Without CI, `check-no-network.ps1` and the tests only run when someone runs them by hand.
