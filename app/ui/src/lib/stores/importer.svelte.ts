// Importing clips from other recorders: the running import's progress, kept here so closing
// the dialog does not lose it, and a message when it is done.

import { toast } from 'svelte-sonner';
import { api, errorMessage } from '$lib/api';
import type { ImportDone, ImportFile, ImportProgress } from '$lib/api/types';

export const importer = $state({
	/** The import dialog, shown from the clips toolbar or the sidebar's progress. */
	dialogOpen: false,
	/** Umbra is dragging its own clips out; the drop overlay stays away. */
	ownDrag: false,
	/** Paths being dragged over the window, while they are. */
	dragging: null as string[] | null,
	/** Paths dropped on the window, for the dialog to add. */
	dropped: [] as string[],
	running: false,
	progress: null as ImportProgress | null,
	result: null as ImportDone | null
});

let listening: Promise<void> | null = null;

/** Listens for the backend's import events; once. */
export function initImporter(): Promise<void> {
	listening ??= (async () => {
		await api.onImportProgress((p) => {
			importer.running = true;
			importer.progress = p;
		});
		await api.onImportDone((d) => {
			importer.running = false;
			importer.progress = null;
			importer.result = d;
			const parts = [`${d.imported} imported`];
			if (d.removed) parts.push(`${d.removed} ${d.removed === 1 ? 'original' : 'originals'} removed after checking`);
			if (d.skipped) parts.push(`${d.skipped} already there`);
			if (d.failed.length) parts.push(`${d.failed.length} with a problem (originals kept)`);
			let description = parts.join(', ') + '.';
			// Why the first one went wrong, so a full disk or a broken file is plain to see.
			if (d.failed.length) description += ` ${d.failed[0]}${d.failed.length > 1 ? ` (and ${d.failed.length - 1} more)` : ''}`;
			const message = d.failed.length ? toast.warning : toast.success;
			message('Import finished', { description, duration: d.failed.length ? 20000 : undefined });
		});
	})();
	return listening;
}

/** How far the import is, 0 to 1: by bytes once known, else by files. */
export function importFraction(p: ImportProgress): number {
	if (p.bytesTotal > 0) return Math.min(1, p.bytesDone / p.bytesTotal);
	return p.total > 0 ? p.done / p.total : 0;
}

/**
 * `about 4 min left`, `about 1 h 20 min left`; null until the speed is known. `short` gives
 * `4 min`, `1 h 20 min` for the sidebar.
 */
export function timeLeft(secs: number | null, short = false): string | null {
	if (secs == null || !Number.isFinite(secs)) return null;
	const mins = Math.round(secs / 60);
	let text: string;
	if (secs < 60) text = short ? '< 1 min' : 'under a minute';
	else if (mins < 60) text = `${mins} min`;
	else text = mins % 60 === 0 ? `${Math.floor(mins / 60)} h` : `${Math.floor(mins / 60)} h ${mins % 60} min`;
	if (short) return text;
	return secs < 60 ? `${text} left` : `about ${text} left`;
}

let dropListening: Promise<void> | null = null;

/** Files dropped on the window open the import with them; once. */
export function initFileDrop(): Promise<void> {
	dropListening ??= (async () => {
		await api.onFileDrop((e) => {
			if (importer.ownDrag) return;
			if (e.type === 'enter') importer.dragging = e.paths;
			else if (e.type === 'leave') importer.dragging = null;
			else if (e.type === 'drop') {
				importer.dragging = null;
				if (e.paths.length === 0) return;
				if (importer.running) {
					toast('An import is running', { description: 'Drop them again once it is done.' });
					importer.dialogOpen = true;
					return;
				}
				importer.dropped = [...importer.dropped, ...e.paths];
				importer.dialogOpen = true;
			}
		});
	})().catch(() => {
		dropListening = null;
	});
	return dropListening;
}

export function openImport() {
	importer.dialogOpen = true;
}

export async function startImport(files: ImportFile[], moveOriginals = false) {
	await initImporter();
	const wanted = files.filter((f) => !f.duplicate).map((f) => ({ path: f.path, game: f.game }));
	if (wanted.length === 0) return;
	importer.result = null;
	importer.running = true;
	importer.progress = { done: 0, total: wanted.length, current: '', bytesDone: 0, bytesTotal: 0, rate: 0, etaSecs: null, active: [], moving: moveOriginals };
	try {
		await api.startImport(wanted, moveOriginals);
	} catch (error) {
		importer.running = false;
		importer.progress = null;
		toast.error('Could not start the import', { description: errorMessage(error) });
	}
}
