// Multi-selection in the library (Clips and album pages). Paths identify clips.

export const selection = $state({
	paths: [] as string[],
	/** The last clip clicked without Shift, where a Shift range starts. */
	anchor: null as string | null
});

export function isSelected(path: string): boolean {
	return selection.paths.includes(path);
}

export function setSelected(path: string, on: boolean) {
	const has = isSelected(path);
	if (on && !has) selection.paths = [...selection.paths, path];
	else if (!on && has) selection.paths = selection.paths.filter((p) => p !== path);
}

export function toggleSelected(path: string) {
	setSelected(path, !isSelected(path));
	selection.anchor = path;
}

/** Shift+click: select everything between the anchor and `path` in the order shown. */
export function selectRange(ordered: string[], path: string) {
	const from = selection.anchor ? ordered.indexOf(selection.anchor) : -1;
	const to = ordered.indexOf(path);
	if (to < 0) return;
	if (from < 0) {
		toggleSelected(path);
		return;
	}
	const [a, b] = from < to ? [from, to] : [to, from];
	const range = ordered.slice(a, b + 1);
	selection.paths = [...new Set([...selection.paths, ...range])];
}

export function selectAll(paths: string[]) {
	selection.paths = [...paths];
}

export function clearSelection() {
	selection.paths = [];
	selection.anchor = null;
}

/** Drops selected paths that are no longer shown (after a filter change or delete). */
export function keepOnly(visible: string[]) {
	const set = new Set(visible);
	const kept = selection.paths.filter((p) => set.has(p));
	if (kept.length !== selection.paths.length) selection.paths = kept;
}
