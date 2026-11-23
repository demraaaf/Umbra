import type { Clip } from '$lib/api/types';
import { clipTitle } from '$lib/format/format';
import type { SortKey } from './view.svelte';

const comparators: Record<SortKey, (a: Clip, b: Clip) => number> = {
	newest: (a, b) => b.createdAt.localeCompare(a.createdAt),
	oldest: (a, b) => a.createdAt.localeCompare(b.createdAt),
	longest: (a, b) => b.durationSecs - a.durationSecs,
	shortest: (a, b) => a.durationSecs - b.durationSecs,
	largest: (a, b) => b.sizeBytes - a.sizeBytes,
	smallest: (a, b) => a.sizeBytes - b.sizeBytes,
	name: (a, b) => clipTitle(a).localeCompare(clipTitle(b), undefined, { numeric: true })
};

export function sortClips(clips: Clip[], key: SortKey): Clip[] {
	return [...clips].sort(comparators[key]);
}
