# Umbra UI

The frontend for Umbra: Svelte 5 (runes) + TypeScript + Tailwind v4 + shadcn-svelte (bits-ui), built as a
static single-page app that Tauri 2 loads from `dist/`. Dark theme only, black/white/grey with red for danger
and green for confirmation (see "Design direction" in `../../plan.md`).

## Run it

```sh
npm install
npm run dev      # http://127.0.0.1:1420, runs in any browser against the in-memory mock
npm run build    # static output in dist/
npm run check    # svelte-check, must report 0 errors and 0 warnings
```

Outside Tauri there is no Rust side, so `src/lib/api/index.ts` picks a rich in-memory mock
(`src/lib/api/mock.ts`): two monitors (2560x1440 at 360 Hz, 1920x1080 at 144 Hz), 24 clips across four games,
a status that ticks while "recording", microphones, audio apps, simulated `saveClip` and export progress, and
thumbnails that arrive late for a few clips. Every screen is usable in a plain browser.

Mock notes:

- Clips have `mock://` video URLs, so the editor plays a clock-driven simulation over the thumbnail instead of a
  video file. Real `<video>` playback is used for any other URL.
- `pickFolder()` cycles through sample folders (a browser has no native folder dialog).
- Settings are kept in `localStorage` (`umbra.mock.settings.v1`); clear it to get the defaults back.

## How the Tauri wiring works

`src/lib/api/index.ts` exports `api: Api`. If `window.__TAURI_INTERNALS__` exists it loads `tauri.ts`, which
calls `invoke()` / `listen()` from `@tauri-apps/api` using the names in `COMMANDS` / `EVENTS`
(`src/lib/api/types.ts`). Otherwise it loads the mock. Only the chosen implementation is loaded.

Commands (JS argument keys are camelCase; Tauri maps them to snake_case Rust parameters):

| Api method | Command | Arguments |
| --- | --- | --- |
| `getSettings` | `get_settings` | none |
| `setSettings` | `set_settings` | `{ settings }` |
| `getStatus` | `get_status` | none |
| `getSystemInfo` | `get_system_info` | none |
| `startRecording` / `stopRecording` | `start_recording` / `stop_recording` | none |
| `saveClip` | `save_clip` | none |
| `listMonitors` / `listWindows` / `listAudioApps` / `listMicrophones` | `list_monitors` / `list_windows` / `list_audio_apps` / `list_microphones` | none |
| `listClips` | `list_clips` | none |
| `deleteClip` | `delete_clip` | `{ path }` |
| `renameClip` | `rename_clip` | `{ path, newName }` |
| `revealClip` | `reveal_clip` | `{ path }` |
| `exportClip` | `export_clip` | `{ request }` |
| `pickFolder` | `pick_folder` | none |

Events (the UI listens; the backend emits):

| Api method | Event | Payload |
| --- | --- | --- |
| `onStatus` | `status` | `Status` |
| `onClipSaved` | `clip-saved` | `Clip` |
| `onExportProgress` | `export-progress` | `ExportProgress` |
| `onClipsChanged` | `clips-changed` | none |

`clips-changed` makes the clip store re-fetch `list_clips` (debounced 150 ms). Emit it after background
thumbnails finish and after save, delete, rename and export. A clip with `thumbnailUrl: null` renders as a plain
dark tile until the next fetch. `clip-saved` and the result of `save_clip` are de-duplicated, so the "Clip saved"
toast shows once per file either way.

Rejected commands can carry a string or an object with `message`; `errorMessage()` in `api/index.ts` turns
either into text for toasts.

### Window chrome

The titlebar is drawn by the app (`components/app/TitleBar.svelte`) and uses `data-tauri-drag-region`. In
`tauri.conf.json`:

```json
{
  "build": {
    "devUrl": "http://127.0.0.1:1420",
    "frontendDist": "../ui/dist",
    "beforeDevCommand": "npm --prefix ../ui run dev",
    "beforeBuildCommand": "npm --prefix ../ui run build"
  },
  "app": {
    "windows": [{ "decorations": false, "minWidth": 900, "minHeight": 560, "width": 1280, "height": 800 }]
  }
}
```

The capability file needs these window permissions for the titlebar (they are not part of `Api`; they go to
`@tauri-apps/api/window`, see `api/window.ts`), plus event listening:

```
core:event:default
core:window:allow-minimize
core:window:allow-toggle-maximize
core:window:allow-close
core:window:allow-is-maximized
core:window:allow-start-dragging
```

The UI makes no network requests. Fonts are bundled (`@fontsource-variable/geist`), scripts are external
modules, there is no `eval`, and `<video>` / `<img>` never set `crossorigin`, so a strict CSP
(`default-src 'self'; img-src 'self' data: blob: asset: http://asset.localhost; media-src ...`) works.

## Layout

```
src/
  main.ts                      entry; disables the webview context menu in release builds
  app.css                      design tokens (palette, 4px spacing, radii, type, motion) and base styles
  App.svelte                   shell: titlebar, sidebar, screen, status strip, toasts
  lib/
    api/                       types.ts (contract), index.ts, tauri.ts, mock.ts, mock-data.ts, window.ts
    stores/                    app.svelte.ts (navigation, settings, status, clips), view.svelte.ts (library view)
    editor/                    playback.svelte.ts, session.svelte.ts (trim/crop/tracks/export), crop.ts
    format/                    format.ts (durations, sizes, fps), hotkey.ts, templates.ts
    screens/                   ClipsScreen, EditorScreen, SettingsScreen
    components/
      ui/                      shadcn-svelte components (source, adapted to the design system)
      app/                     TitleBar, Sidebar, StatusStrip, IconButton
        clips/ editor/ settings/ fields/
```

## Design system notes

- All tokens live in `src/app.css`: `--surface-*`, `--line*`, `--text*`, `--danger*`, `--success*`, the
  `--space-*` scale (Tailwind's `--spacing` is 4px too), radii (2 to 8px), type sizes (12/13/14/16/18/22) and
  motion (`--dur-*`, `--ease-out`). Change a colour there and everything follows.
- The shadcn-svelte "nova" components are kept as source and adapted: denser heights (28px controls), the 4px
  grid, no blur or halo, crisper focus rings. `app.css` also aliases `data-checked`, `data-open` and friends to
  bits-ui's `data-state` attributes, which the components otherwise would not match.
- Motion is functional and 120 to 180 ms, with `prefers-reduced-motion` collapsing transforms and keyframes.
- Timecodes and numbers use `tabular-nums`.

## Editor shortcuts

`Space` play/pause, `J` `K` `L` shuttle, `Left` `Right` step a frame (`Shift` for a second), `I` `O` set the
start and end at the playhead, `Home` `End` jump to the trim points, `Esc` back to the library. Trim handles and
the crop box are keyboard operable, and the numeric fields in the inspector do the same job without dragging.
`Ctrl`+wheel zooms the timeline; the wheel pans it when zoomed.
