// The clips in one edit. The editor opens with one clip; the media pool adds more after it,
// each keeping its own edits (its session). The first clip's session holds what is about the
// whole edit: export settings, file name and fades. Export joins them in this order.

import { app } from '$lib/stores/app.svelte';
import type { Clip } from '$lib/api/types';
import { getSession, type EditSession } from './session.svelte';

export class Sequence {
	/** Clip paths in order; the first is the clip the editor was opened with. */
	items = $state<string[]>([]);
	/** The clip being edited, by index. */
	active = $state(0);
	/** Playing carries on into the next clip at the end of one. */
	playAll = $state(false);

	constructor(first: string) {
		this.items = [first];
	}

	/** The clips that are still in the library, with their sessions. */
	get entries(): { clip: Clip; session: EditSession }[] {
		return this.items
			.map((path) => app.clips.find((c) => c.path === path))
			.filter((c): c is Clip => !!c)
			.map((clip) => ({ clip, session: getSession(clip) }));
	}

	get activeEntry(): { clip: Clip; session: EditSession } | null {
		const entries = this.entries;
		return entries[Math.min(this.active, entries.length - 1)] ?? null;
	}

	get main(): EditSession | null {
		return this.entries[0]?.session ?? null;
	}

	/** Length of the whole edit in seconds. */
	get totalSecs(): number {
		return this.entries.reduce((sum, e) => sum + e.session.outputSecs, 0);
	}

	/** Where each clip starts in the whole edit, in seconds. */
	get offsets(): number[] {
		let at = 0;
		return this.entries.map((e) => {
			const start = at;
			at += e.session.outputSecs;
			return start;
		});
	}

	has(path: string): boolean {
		return this.items.includes(path);
	}

	/** Adds a clip after the one being edited (or at `index`). */
	add(path: string, index?: number) {
		if (this.has(path)) return;
		const at = Math.min(this.items.length, Math.max(1, index ?? this.active + 1));
		this.items = [...this.items.slice(0, at), path, ...this.items.slice(at)];
	}

	remove(index: number) {
		// The first clip is the edit itself; it stays.
		if (index <= 0 || index >= this.items.length) return;
		this.items = this.items.filter((_, i) => i !== index);
		if (this.active >= this.items.length) this.active = this.items.length - 1;
		else if (this.active > index) this.active -= 1;
	}

	/** Moves a clip to another place; the first stays first. */
	move(from: number, to: number) {
		if (from <= 0 || to <= 0 || from === to || from >= this.items.length) return;
		const next = [...this.items];
		const [path] = next.splice(from, 1);
		next.splice(Math.min(to, next.length), 0, path);
		const activePath = this.items[this.active];
		this.items = next;
		this.active = Math.max(0, next.indexOf(activePath));
	}

	select(index: number) {
		if (index < 0 || index >= this.items.length || index === this.active) return;
		this.activeEntry?.session.playback.pause();
		this.active = index;
	}
}

const sequences = new Map<string, Sequence>();

export function getSequence(path: string): Sequence {
	let sequence = sequences.get(path);
	if (!sequence) {
		sequence = new Sequence(path);
		sequences.set(path, sequence);
	}
	return sequence;
}

/** How much of the picture shows at time `t` of the whole edit (0 black, 1 as it is). */
export function fadeAt(fades: { fadeIn: number; fadeOut: number; between: number }, t: number, total: number, joins: number[]): number {
	let level = 1;
	if (fades.fadeIn > 0 && t < fades.fadeIn) level = Math.min(level, t / fades.fadeIn);
	if (fades.fadeOut > 0 && total - t < fades.fadeOut) level = Math.min(level, (total - t) / fades.fadeOut);
	if (fades.between > 0) {
		for (const join of joins) {
			const d = Math.abs(t - join);
			if (d < fades.between) level = Math.min(level, d / fades.between);
		}
	}
	return Math.min(1, Math.max(0, level));
}
