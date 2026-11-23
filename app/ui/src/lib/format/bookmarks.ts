// Bookmarks in a recording: moments marked with the clip hotkey while it recorded.

/** Within this much of a bookmark counts as being on it. */
const NEAR_SECS = 0.25;

/** The bookmark after (`1`) or before (`-1`) `time`, or null if there is none. */
export function stepBookmark(bookmarks: number[], time: number, direction: -1 | 1): number | null {
	const sorted = [...bookmarks].sort((a, b) => a - b);
	if (direction > 0) return sorted.find((b) => b > time + NEAR_SECS) ?? null;
	return sorted.reverse().find((b) => b < time - NEAR_SECS) ?? null;
}

/** A bookmark within `tolerance` seconds of `time`, to snap a click onto it. */
export function nearBookmark(bookmarks: number[], time: number, tolerance: number): number | null {
	let best: number | null = null;
	for (const b of bookmarks) {
		if (Math.abs(b - time) <= tolerance && (best === null || Math.abs(b - time) < Math.abs(best - time))) best = b;
	}
	return best;
}
