// Application state shared by every screen: navigation, settings, live status, the clip library
// and albums. Screens read from `app` and call the functions below; nothing else talks to `api`
// for these concerns, so the mock and the real backend behave the same from here on.

import { toast } from 'svelte-sonner';
import { api, errorMessage } from '$lib/api';
import type {
	Album,
	BurnLayers,
	BurnResult,
	CaptureTarget,
	Clip,
	ProcessingItem,
	ShrinkQuality,
	ShrinkResult,
	RecordingFinishing,
	RecordingKind,
	RecordingSaved,
	Settings,
	Status,
	SystemInfo
} from '$lib/api/types';
import { FAVORITES } from '$lib/api/types';
import { clipTitle, formatBytes, formatDuration, gameLabel, plural } from '$lib/format/format';
import { clearSelection } from './selection.svelte';
import { openSettingsPage } from './settingsNav.svelte';

export type Screen = 'clips' | 'player' | 'editor' | 'viewer' | 'albums' | 'album' | 'settings';

/** Lets the editor's session registry follow clips that are renamed or removed, without a circular import. */
export const clipHooks: {
	moved?: (oldPath: string, clip: Clip) => void;
	removed?: (path: string) => void;
} = {};

export const app = $state({
	screen: 'clips' as Screen,
	/** The clip open in the editor. Kept when leaving the editor so it can be returned to. */
	editorPath: null as string | null,
	/** Where the editor should put the playhead when it next shows its clip, in seconds. */
	editorSeek: null as number | null,
	/** The clip open in the player. */
	playerPath: null as string | null,
	/** The screenshot open in the image viewer. */
	viewerPath: null as string | null,
	/** The library list a clip was opened from, in its order, for Ctrl and the arrow keys. */
	browse: [] as string[],
	/** The clip whose Properties dialog is open. */
	propertiesPath: null as string | null,
	/** The album page being shown. */
	albumId: null as string | null,
	/** Where the editor, player and viewer return to. */
	returnTo: 'clips' as 'clips' | 'album',

	/** Settings as the backend stores them. */
	settings: null as Settings | null,
	/** Working copy edited on the Settings screen; applied with `applyDraft()`. */
	draft: null as Settings | null,
	status: null as Status | null,
	systemInfo: null as SystemInfo | null,

	clips: [] as Clip[],
	clipsState: 'loading' as 'loading' | 'ready' | 'error',
	clipsError: null as string | null,

	albums: [] as Album[],
	albumsState: 'loading' as 'loading' | 'ready' | 'error',

	saving: false,
	shooting: false,
	stoppingFile: false,
	/** Clips being saved: from the hotkey until the file is there. */
	savingClips: [] as SavingClip[],
	/** Files Umbra is still working on, such as a long recording being made quick to open. */
	processing: [] as ProcessingItem[],
	/** Umbra has not been set up on this PC yet: the first-start screen shows. */
	firstRun: false,
	togglingRecording: false,
	booted: false,
	fatal: null as string | null
});

// ---------- Navigation ----------

/** Opens Settings at one section, such as `help`. */
export function openSettingsAt(section: string) {
	openSettingsPage(section);
	go('settings');
}

export function go(screen: Screen) {
	if (screen === 'editor' && !app.editorPath) return;
	if (screen !== app.screen) clearSelection();
	app.screen = screen;
}

/**
 * Opens a clip to look at: a video in the player, a screenshot in the viewer. `list` is the
 * library's order, so the player can step to the previous and next item.
 */
export function openClip(path: string, list?: string[]) {
	const clip = app.clips.find((c) => c.path === path);
	if (app.screen === 'clips' || app.screen === 'album') app.returnTo = app.screen;
	if (list) app.browse = list;
	if (clip?.kind === 'image') {
		app.viewerPath = path;
		app.screen = 'viewer';
	} else {
		app.playerPath = path;
		app.screen = 'player';
	}
}

export function showProperties(path: string) {
	app.propertiesPath = path;
}

export function closeProperties() {
	app.propertiesPath = null;
}

/** Opens a clip in the editor, with the playhead at `atSecs` when given. */
export function openInEditor(path: string, atSecs?: number) {
	if (app.clips.find((c) => c.path === path)?.kind === 'image') return;
	if (app.screen === 'clips' || app.screen === 'album') app.returnTo = app.screen;
	app.editorPath = path;
	app.editorSeek = atSecs ?? null;
	app.screen = 'editor';
}

/**
 * The last clip opened with a single click. The library hands the click to the player at
 * once, so a second click lands on the player; within the double-click time and near the
 * same spot, that second click opens the editor instead.
 */
let lastOpen: { path: string; at: number; x: number; y: number } | null = null;

export function noteOpenClick(path: string, event: MouseEvent) {
	lastOpen = { path, at: performance.now(), x: event.clientX, y: event.clientY };
}

/** True (and consumed) when `event` completes a double-click on the clip just opened. */
export function takeDoubleClick(event: MouseEvent): string | null {
	const last = lastOpen;
	if (!last || event.button !== 0) return null;
	lastOpen = null;
	const near = Math.abs(event.clientX - last.x) <= 6 && Math.abs(event.clientY - last.y) <= 6;
	return near && performance.now() - last.at <= 500 ? last.path : null;
}

/**
 * Swallows the click that follows a double-click hand-off, so it does not also press
 * whatever the new screen shows under the pointer.
 */
export function swallowNextClick() {
	const stop = (e: MouseEvent) => {
		e.stopPropagation();
		e.preventDefault();
		window.removeEventListener('click', stop, true);
	};
	window.addEventListener('click', stop, true);
	setTimeout(() => window.removeEventListener('click', stop, true), 700);
}

/** Opens the previous (-1) or next (1) item of the list the open clip came from. */
export function browseStep(direction: -1 | 1) {
	const current = app.screen === 'viewer' ? app.viewerPath : app.playerPath;
	const existing = new Set(app.clips.map((c) => c.path));
	const list = app.browse.filter((p) => existing.has(p));
	const index = current ? list.indexOf(current) : -1;
	const next = list[index + direction];
	if (index < 0 || !next) return false;
	openClip(next);
	return true;
}

export function browsePosition(): { index: number; count: number } {
	const current = app.screen === 'viewer' ? app.viewerPath : app.playerPath;
	const existing = new Set(app.clips.map((c) => c.path));
	const list = app.browse.filter((p) => existing.has(p));
	return { index: current ? list.indexOf(current) : -1, count: list.length };
}

export function openAlbum(id: string) {
	clearSelection();
	app.albumId = id;
	app.screen = 'album';
}

/** Back from the editor or viewer to the library view the clip was opened from. */
export function backToLibrary() {
	app.screen = app.returnTo === 'album' && app.albums.some((a) => a.id === app.albumId) ? 'album' : 'clips';
}

/** Kept for existing callers. */
export const closeEditor = backToLibrary;

export function currentClip(): Clip | null {
	return app.clips.find((c) => c.path === app.editorPath) ?? null;
}

export function playerClip(): Clip | null {
	return app.clips.find((c) => c.path === app.playerPath) ?? null;
}

export function viewerClip(): Clip | null {
	return app.clips.find((c) => c.path === app.viewerPath) ?? null;
}

export function currentAlbum(): Album | null {
	return app.albums.find((a) => a.id === app.albumId) ?? null;
}

// ---------- Clip library ----------

let refreshTimer: ReturnType<typeof setTimeout> | null = null;

export async function refreshClips() {
	try {
		// A clip waiting out its undo time stays hidden.
		app.clips = (await api.listClips()).filter((c) => !pendingDeletes.has(c.path));
		app.clipsState = 'ready';
		app.clipsError = null;
	} catch (error) {
		app.clipsError = errorMessage(error);
		app.clipsState = 'error';
	}
}

/**
 * The backend emits `clips-changed` after background thumbnails finish and after a save, delete,
 * rename or export. Bursts are folded into one re-fetch.
 */
function scheduleClipsRefresh() {
	if (refreshTimer) clearTimeout(refreshTimer);
	refreshTimer = setTimeout(() => {
		refreshTimer = null;
		void refreshClips();
	}, 150);
}

export function upsertClip(clip: Clip) {
	const index = app.clips.findIndex((c) => c.path === clip.path);
	if (index >= 0) app.clips[index] = clip;
	else app.clips = [clip, ...app.clips];
}

const announced = new Set<string>();

/** Called for the `clip-saved` event and for the result of `saveClip()`; toasts once per file. */
/** A clip on its way, shown as a placeholder in the library. */
export interface SavingClip {
	id: number;
	game: string;
	waitSecs: number;
	lengthSecs: number;
	/** `performance.now()` when the hotkey was pressed. */
	started: number;
}

let savingId = 0;

function handleClipSaving(event: { game: string; waitSecs: number; lengthSecs: number }) {
	const id = ++savingId;
	// Just after recording started the buffer holds less than a full clip.
	const held = app.status?.state === 'recording' ? Math.round(app.status.bufferSecs) + event.waitSecs : event.lengthSecs;
	const lengthSecs = Math.max(1, Math.min(event.lengthSecs, held));
	app.savingClips = [...app.savingClips, { id, ...event, lengthSecs, started: performance.now() }];
	// Never left behind if no answer comes.
	setTimeout(() => (app.savingClips = app.savingClips.filter((c) => c.id !== id)), (event.waitSecs + 60) * 1000);
}

/** The oldest clip on its way is done (saved or failed). */
function clipArrived() {
	app.savingClips = app.savingClips.slice(1);
}

export function handleClipSaved(clip: Clip) {
	if (clip.kind !== 'image') clipArrived();
	upsertClip(clip);
	if (announced.has(clip.path)) return;
	announced.add(clip.path);
	const image = clip.kind === 'image';
	toast.success(image ? 'Screenshot saved' : 'Clip saved', {
		description: image
			? `${gameLabel(clip.game)} · ${clip.width}×${clip.height} · ${formatBytes(clip.sizeBytes)}`
			: `${gameLabel(clip.game)} · ${formatDuration(clip.durationSecs)} · ${formatBytes(clip.sizeBytes)}`,
		action: { label: 'Open', onClick: () => openClip(clip.path) }
	});
}

export async function saveClipNow() {
	if (app.saving) return;
	app.saving = true;
	try {
		handleClipSaved(await api.saveClip());
	} catch (error) {
		toast.error('Could not save the clip', { description: errorMessage(error) });
	} finally {
		app.saving = false;
	}
}

// ---------- Recordings ----------

/** Compresses clips now (right-click or the selection). */
export async function compressClips(paths: string[], quality: ShrinkQuality | null = null) {
	try {
		const { queued, waiting } = await api.compressClips(paths, quality);
		if (queued === 0) {
			toast('Already being compressed', { description: 'Those clips are in line already.' });
			return;
		}
		toast(queued === 1 ? 'Compressing the clip' : `Compressing ${queued} clips`, {
			description: waiting
				? 'It starts once the game is closed and nothing is being recorded, so it never slows a game down.'
				: 'In the background, one after another. The library shows how far it is.',
			action: { label: 'Stop', onClick: () => void stopCompressing(paths) }
		});
	} catch (error) {
		toast.error('Could not compress', { description: errorMessage(error) });
	}
}

/** Stops compressing clips: the one being compressed and any waiting. They stay as they are. */
export async function stopCompressing(paths: string[]) {
	try {
		await api.stopCompressing(paths);
	} catch (error) {
		toast.error('Could not stop compressing', { description: errorMessage(error) });
	}
}

function handleShrinkResult(result: ShrinkResult) {
	const name = result.fileName.replace(/[.]mp4$/i, '');
	if (result.stopped) {
		toast('Stopped compressing', {
			id: 'compress-stopped',
			description: `${name} stays as it is, and is not compressed automatically either.`
		});
	} else if (result.savedBytes !== null) {
		toast.success('Compressed', { description: `${name} · ${formatBytes(result.savedBytes)} smaller` });
	} else if (result.problem) {
		toast(`${name} stays as it is`, { description: `${result.problem[0].toUpperCase()}${result.problem.slice(1)}.` });
	}
}

// ---------- Burn in ----------

/** The layers any of these clips has. */
export function layersOf(paths: string[]): BurnLayers {
	const clips = app.clips.filter((c) => paths.includes(c.path));
	return {
		camera: clips.some((c) => c.camera !== null),
		input: clips.some((c) => c.input),
		logo: clips.some((c) => c.logo)
	};
}

/** Burns layers into clips, one after another in the background. Clips that have none of them are left out. */
export async function burnIn(paths: string[], layers: BurnLayers) {
	const targets = app.clips
		.filter((c) => paths.includes(c.path))
		.filter((c) => (layers.camera && c.camera !== null) || (layers.input && c.input) || (layers.logo && c.logo))
		.map((c) => c.path);
	try {
		const queued = await api.burnIn(targets, layers);
		if (queued === 0) {
			toast('Already in line', { description: 'Those clips are being burned in already.' });
			return;
		}
		const replace = app.settings?.export.burnReplaceOriginal ?? false;
		toast(queued === 1 ? 'Burning in' : `Burning in ${queued} clips`, {
			description: replace
				? 'In the background, one after another. Each takes its original’s place once it checks out.'
				: 'In the background, one after another. Each is a new file next to its original.',
			action: { label: 'Stop', onClick: () => void stopBurnIn(targets) }
		});
	} catch (error) {
		toast.error('Could not burn in', { description: errorMessage(error) });
	}
}

/** Stops burning into clips (or into their new files): the one at work and any waiting. */
export async function stopBurnIn(paths: string[]) {
	try {
		await api.stopBurnIn(paths);
	} catch (error) {
		toast.error('Could not stop', { description: errorMessage(error) });
	}
}

function handleBurnResult(result: BurnResult) {
	const name = result.fileName.replace(/[.]mp4$/i, '');
	if (result.stopped) {
		toast('Stopped burning in', { id: 'burn-stopped', description: `${name} stays as it is.` });
	} else if (result.output) {
		const replaced = result.output === result.path;
		toast.success('Burned in', {
			description: replaced ? `${name} now has its layers in the picture.` : `Saved as ${result.output.split(/[\\/]/).pop()?.replace(/[.]mp4$/i, '')}.`
		});
	} else if (result.problem) {
		toast(`Could not burn into ${name}`, { description: `${result.problem[0].toUpperCase()}${result.problem.slice(1)}.` });
	}
}

export async function startFileRecording(kind: RecordingKind = 'manual', source: CaptureTarget | null = null) {
	try {
		await api.startFileRecording(kind, source);
	} catch (error) {
		toast.error('Could not start recording', { description: errorMessage(error) });
	}
}

export async function stopFileRecording() {
	if (app.stoppingFile) return;
	app.stoppingFile = true;
	try {
		await api.stopFileRecording();
	} catch (error) {
		toast.error('Could not stop the recording', { description: errorMessage(error) });
	} finally {
		app.stoppingFile = false;
	}
}

/** Puts clips on the clipboard as files, ready to paste into Discord or a folder. */
export async function copyClipFiles(paths: string[]) {
	// A clip with a camera or logo is exported with them drawn in first, which takes a moment.
	const id = toast.loading('Copying…');
	try {
		await api.copyClipsToClipboard(paths);
		toast.success(paths.length === 1 ? 'File copied' : `${paths.length} files copied`, {
			id,
			description: 'Paste into Discord, a chat or a folder with Ctrl+V.'
		});
	} catch (error) {
		toast.error('Could not copy the file', { id, description: errorMessage(error) });
	}
}

export async function pauseFileRecording(paused: boolean) {
	try {
		await api.pauseFileRecording(paused);
	} catch (error) {
		toast.error(paused ? 'Could not pause the recording' : 'Could not resume the recording', {
			description: errorMessage(error)
		});
	}
}

/** One message per recording: "finishing" turns into "saved" in place. */
const FINISH_TOAST = 'recording-finish';

let processingTimer: ReturnType<typeof setTimeout> | null = null;

/** Reads the files being worked on, and again every second while there are any. */
export async function refreshProcessing() {
	if (processingTimer) clearTimeout(processingTimer);
	processingTimer = null;
	try {
		app.processing = await api.listProcessing();
	} catch {
		app.processing = [];
	}
	if (app.processing.length > 0) processingTimer = setTimeout(refreshProcessing, 1000);
}

function handleRecordingFinishing(event: RecordingFinishing) {
	void refreshProcessing();
	toast.loading(event.kind === 'session' ? 'Finishing the session recording' : 'Finishing the recording', {
		id: FINISH_TOAST,
		description: `${event.game} · making it quick to open; long recordings take a moment`
	});
}

function handleRecordingSaved(saved: RecordingSaved) {
	const what = saved.kind === 'session' ? 'Session recording' : 'Recording';
	if (!saved.clip) {
		toast.error(`${what} not saved`, { id: FINISH_TOAST, description: saved.error ?? undefined });
		return;
	}
	const clip = saved.clip;
	upsertClip(clip);
	announced.add(clip.path);
	const marks = clip.bookmarks.length;
	toast.success(`${what} saved`, {
		description: [
			`${gameLabel(clip.game)} · ${formatDuration(clip.durationSecs)} · ${formatBytes(clip.sizeBytes)}`,
			marks ? plural(marks, 'bookmark') : '',
			saved.error ? `It ended early: ${saved.error}` : ''
		]
			.filter(Boolean)
			.join(' · '),
		action: { label: 'Open', onClick: () => openClip(clip.path) },
		id: FINISH_TOAST
	});
}

export async function takeScreenshotNow() {
	if (app.shooting) return;
	app.shooting = true;
	try {
		handleClipSaved(await api.takeScreenshot());
	} catch (error) {
		toast.error('Could not take the screenshot', { description: errorMessage(error) });
	} finally {
		app.shooting = false;
	}
}

function forget(paths: string[]) {
	const gone = new Set(paths);
	paths.forEach((p) => clipHooks.removed?.(p));
	app.clips = app.clips.filter((c) => !gone.has(c.path));
	app.albums = app.albums.map((a) => ({ ...a, clipPaths: a.clipPaths.filter((p) => !gone.has(p)) }));
	const open = [app.editorPath, app.viewerPath, app.playerPath].some((p) => p && gone.has(p));
	if (open) {
		if (app.editorPath && gone.has(app.editorPath)) app.editorPath = null;
		if (app.viewerPath && gone.has(app.viewerPath)) app.viewerPath = null;
		if (app.playerPath && gone.has(app.playerPath)) app.playerPath = null;
		if (['editor', 'viewer', 'player'].includes(app.screen)) backToLibrary();
	}
}

/** Deleting skips the Recycle Bin, so the messages must not promise it. */
export function skipsBin(): boolean {
	return !!app.settings?.storage.skipRecycleBin;
}

export async function deleteClip(path: string): Promise<boolean> {
	try {
		await api.deleteClip(path);
		forget([path]);
		toast(skipsBin() ? 'Deleted' : 'Moved to the Recycle Bin');
		return true;
	} catch (error) {
		toast.error('Could not delete the clip', { description: errorMessage(error) });
		return false;
	}
}

// ---------- Delete with undo ----------
// The player and the viewer delete on a single key press. The clip disappears at once and the
// next one opens; the file only goes to the Recycle Bin once the undo time is up.

const UNDO_MS = 5000;
const pendingDeletes = new Map<string, { clip: Clip; index: number; timer: ReturnType<typeof setTimeout> }>();

export function deleteWithUndo(path: string) {
	const index = app.clips.findIndex((c) => c.path === path);
	if (index < 0 || pendingDeletes.has(path)) return;
	const clip = app.clips[index];
	// Move on first, while the clip is still in the list: the next one, else the one before.
	const moved = browseStep(1) || browseStep(-1);
	app.clips = app.clips.filter((c) => c.path !== path);
	if (!moved) {
		if (app.playerPath === path) app.playerPath = null;
		if (app.viewerPath === path) app.viewerPath = null;
		backToLibrary();
	}
	const timer = setTimeout(() => void commitDelete(path), UNDO_MS);
	pendingDeletes.set(path, { clip, index, timer });
	toast(`Deleted ${clipTitle(clip)}`, {
		description: skipsBin() ? 'After that it is deleted for good.' : 'It goes to the Recycle Bin.',
		duration: UNDO_MS,
		action: { label: 'Undo', onClick: () => undoDelete(path) }
	});
}

function restorePending(entry: { clip: Clip; index: number }) {
	if (app.clips.some((c) => c.path === entry.clip.path)) return;
	const list = [...app.clips];
	list.splice(Math.min(entry.index, list.length), 0, entry.clip);
	app.clips = list;
}

function undoDelete(path: string) {
	const entry = pendingDeletes.get(path);
	if (!entry) return;
	clearTimeout(entry.timer);
	pendingDeletes.delete(path);
	restorePending(entry);
	openClip(path);
}

async function commitDelete(path: string) {
	const entry = pendingDeletes.get(path);
	if (!entry) return;
	pendingDeletes.delete(path);
	try {
		await api.deleteClip(path);
		forget([path]);
	} catch (error) {
		restorePending(entry);
		toast.error('Could not delete the clip', { description: errorMessage(error) });
	}
}

export async function deleteClips(paths: string[]): Promise<boolean> {
	if (paths.length === 0) return true;
	try {
		await api.deleteClips(paths);
		forget(paths);
		clearSelection();
		toast(skipsBin() ? `${plural(paths.length, 'item')} deleted` : `${plural(paths.length, 'item')} moved to the Recycle Bin`);
		return true;
	} catch (error) {
		toast.error('Could not delete the selection', { description: errorMessage(error) });
		void refreshClips();
		return false;
	}
}

export async function renameClip(path: string, newName: string): Promise<Clip | null> {
	try {
		const renamed = await api.renameClip(path, newName);
		clipHooks.moved?.(path, renamed);
		app.clips = app.clips.map((c) => (c.path === path ? renamed : c));
		app.albums = app.albums.map((a) => ({ ...a, clipPaths: a.clipPaths.map((p) => (p === path ? renamed.path : p)) }));
		if (app.editorPath === path) app.editorPath = renamed.path;
		if (app.viewerPath === path) app.viewerPath = renamed.path;
		return renamed;
	} catch (error) {
		toast.error('Could not rename it', { description: errorMessage(error) });
		return null;
	}
}

export async function revealClip(path: string) {
	try {
		await api.revealClip(path);
	} catch (error) {
		toast.error('Could not open the folder', { description: errorMessage(error) });
	}
}

// ---------- Albums ----------

let albumsTimer: ReturnType<typeof setTimeout> | null = null;

export async function refreshAlbums() {
	try {
		app.albums = await api.listAlbums();
		app.albumsState = 'ready';
	} catch (error) {
		app.albumsState = 'error';
		toast.error('Albums could not be loaded', { description: errorMessage(error) });
	}
}

function scheduleAlbumsRefresh() {
	if (albumsTimer) clearTimeout(albumsTimer);
	albumsTimer = setTimeout(() => {
		albumsTimer = null;
		void refreshAlbums();
	}, 150);
}

function adoptAlbum(album: Album) {
	const index = app.albums.findIndex((a) => a.id === album.id);
	if (index >= 0) app.albums[index] = album;
	else app.albums = [...app.albums, album];
}

export async function createAlbum(name: string): Promise<Album | null> {
	try {
		const album = await api.createAlbum(name);
		adoptAlbum(album);
		return album;
	} catch (error) {
		toast.error('Could not create the album', { description: errorMessage(error) });
		return null;
	}
}

export async function renameAlbum(id: string, name: string): Promise<boolean> {
	try {
		adoptAlbum(await api.renameAlbum(id, name));
		return true;
	} catch (error) {
		toast.error('Could not rename the album', { description: errorMessage(error) });
		return false;
	}
}

export async function deleteAlbum(id: string): Promise<boolean> {
	try {
		await api.deleteAlbum(id);
		app.albums = app.albums.filter((a) => a.id !== id);
		if (app.albumId === id && app.screen === 'album') app.screen = 'albums';
		toast('Album deleted. Its clips are still in the library.');
		return true;
	} catch (error) {
		toast.error('Could not delete the album', { description: errorMessage(error) });
		return false;
	}
}

export async function setAlbumHidden(id: string, hidden: boolean) {
	try {
		adoptAlbum(await api.setAlbumHidden(id, hidden));
	} catch (error) {
		toast.error('Could not change the album', { description: errorMessage(error) });
	}
}

export async function addToAlbum(id: string, paths: string[]): Promise<boolean> {
	if (paths.length === 0) return false;
	try {
		const album = await api.addToAlbum(id, paths);
		adoptAlbum(album);
		toast.success(`Added ${plural(paths.length, 'item')} to ${album.name}`);
		scheduleClipsRefresh();
		return true;
	} catch (error) {
		toast.error('Could not add to the album', { description: errorMessage(error) });
		return false;
	}
}

export async function removeFromAlbum(id: string, paths: string[]): Promise<boolean> {
	try {
		const album = await api.removeFromAlbum(id, paths);
		adoptAlbum(album);
		clearSelection();
		toast(`Removed ${plural(paths.length, 'item')} from ${album.name}`);
		scheduleClipsRefresh();
		return true;
	} catch (error) {
		toast.error('Could not remove from the album', { description: errorMessage(error) });
		return false;
	}
}

export function isFavorite(clip: Clip): boolean {
	return clip.albums.includes(FAVORITES);
}

/** Hearts or un-hearts a clip (adds it to Favorites or takes it out). The heart itself is the feedback. */
export async function toggleFavorite(path: string) {
	const clip = app.clips.find((c) => c.path === path);
	if (!clip) return;
	const on = !isFavorite(clip);
	const mark = (favorite: boolean) => {
		app.clips = app.clips.map((c) =>
			c.path === path
				? { ...c, albums: favorite ? [...c.albums.filter((a) => a !== FAVORITES), FAVORITES] : c.albums.filter((a) => a !== FAVORITES) }
				: c
		);
	};
	mark(on);
	try {
		adoptAlbum(on ? await api.addToAlbum(FAVORITES, [path]) : await api.removeFromAlbum(FAVORITES, [path]));
	} catch (error) {
		mark(!on);
		toast.error(on ? 'Could not add to Favorites' : 'Could not take it out of Favorites', { description: errorMessage(error) });
	}
}

/** Paths of clips that belong to an album marked "Hide these clips from Clips". */
export function hiddenPaths(): Set<string> {
	const hidden = new Set<string>();
	for (const album of app.albums) if (album.hideFromClips) album.clipPaths.forEach((p) => hidden.add(p));
	return hidden;
}

export function albumCover(album: Album): string | null {
	for (const path of album.clipPaths) {
		const clip = app.clips.find((c) => c.path === path);
		if (clip?.thumbnailUrl) return clip.thumbnailUrl;
	}
	return null;
}

export function albumTitleList(clip: Clip): string {
	return app.albums
		.filter((a) => a.clipPaths.includes(clip.path))
		.map((a) => a.name)
		.join(', ');
}

// ---------- Recording ----------

export async function toggleRecording() {
	if (app.togglingRecording) return;
	app.togglingRecording = true;
	try {
		if (app.status?.state === 'recording') await api.stopRecording();
		else await api.startRecording();
		app.status = await api.getStatus();
	} catch (error) {
		toast.error('Could not change recording', { description: errorMessage(error) });
	} finally {
		app.togglingRecording = false;
	}
}

// ---------- Settings ----------

const plain = <T>(value: T): T => JSON.parse(JSON.stringify(value)) as T;

export function resetDraft() {
	app.draft = app.settings ? plain(app.settings) : null;
}

export function draftIsDirty(): boolean {
	if (!app.draft || !app.settings) return false;
	return JSON.stringify(app.draft) !== JSON.stringify(app.settings);
}

/** Sends the draft to the backend and adopts what comes back (it may have been adjusted). */
export async function applyDraft(): Promise<void> {
	if (!app.draft) return;
	const stored = await api.setSettings(plain(app.draft));
	app.settings = stored;
	app.draft = plain(stored);
}

/**
 * Applies one change right away (used by the recording panel). Unapplied edits on the Settings
 * screen are kept: the same change is made to the draft instead of replacing it.
 */
export async function patchSettings(mutate: (settings: Settings) => void): Promise<boolean> {
	if (!app.settings) return false;
	const next = plain(app.settings);
	mutate(next);
	try {
		const wasDirty = draftIsDirty();
		const stored = await api.setSettings(next);
		app.settings = stored;
		if (app.draft && wasDirty) mutate(app.draft);
		else app.draft = plain(stored);
		return true;
	} catch (error) {
		toast.error('Could not change the setting', { description: errorMessage(error) });
		return false;
	}
}

export function setCaptureTarget(target: CaptureTarget) {
	return patchSettings((s) => {
		s.capture = plain(target);
	});
}

/**
 * Takes a new status. The camera's hotkey and the tray switch the camera without the window,
 * so a change there is carried into the settings (and the draft) here.
 */
function adoptStatus(status: Status) {
	const before = app.status?.cameraOn;
	app.status = status;
	if (before === undefined || before === status.cameraOn) return;
	if (app.settings && app.settings.camera.enabled !== status.cameraOn) {
		app.settings.camera.enabled = status.cameraOn;
		if (app.draft) app.draft.camera.enabled = status.cameraOn;
	}
}

// ---------- Boot ----------

/** Loads everything the shell needs and subscribes to backend events. Returns a cleanup. */
export async function boot(): Promise<() => void> {
	const disposers: Array<() => void> = [];
	try {
		const [settings, status, systemInfo, firstRun] = await Promise.all([
			api.getSettings(),
			api.getStatus(),
			api.getSystemInfo(),
			api.isFirstRun()
		]);
		app.firstRun = firstRun;
		app.settings = settings;
		app.draft = plain(settings);
		app.status = status;
		app.systemInfo = systemInfo;

		disposers.push(await api.onStatus(adoptStatus));
		disposers.push(await api.onClipSaved(handleClipSaved));
		disposers.push(await api.onClipSaving(handleClipSaving));
		disposers.push(
			await api.onClipFailed((e) => {
				clipArrived();
				toast.error('Clip not saved', { description: e.error });
			})
		);
		disposers.push(await api.onRecordingSaved(handleRecordingSaved));
		disposers.push(await api.onRecordingFinishing(handleRecordingFinishing));
		disposers.push(await api.onProcessingStarted(() => void refreshProcessing()));
		disposers.push(await api.onShrinkResult(handleShrinkResult));
		disposers.push(await api.onBurnResult(handleBurnResult));
		void refreshProcessing();
		disposers.push(
			await api.onStorageCleaned((c) =>
				toast(`Storage limit: ${plural(c.files, 'clip')} removed`, {
					description: `${formatBytes(c.bytes)}, the oldest that were not kept. Change the limits in Settings, Storage.`
				})
			)
		);
		disposers.push(
			await api.onLowDisk((e) => toast.error(e.title, { description: e.detail, duration: 15_000 }))
		);
		disposers.push(await api.onClipsChanged(scheduleClipsRefresh));
		disposers.push(await api.onAlbumsChanged(scheduleAlbumsRefresh));
		app.booted = true;
		void refreshClips();
		void refreshAlbums();
	} catch (error) {
		app.fatal = errorMessage(error);
		app.booted = true;
	}
	return () => {
		disposers.forEach((d) => d());
		if (refreshTimer) clearTimeout(refreshTimer);
		if (albumsTimer) clearTimeout(albumsTimer);
	};
}

