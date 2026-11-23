// Library view options: search, filter, sort and layout. Sort and layout survive restarts.

export type SortKey = 'newest' | 'oldest' | 'longest' | 'shortest' | 'largest' | 'smallest' | 'name';
export type ViewMode = 'grid' | 'list';
/** `video` is clips only; `recording` is recordings and full sessions. */
export type KindFilter = 'all' | 'video' | 'recording' | 'image';
/** Clips with a layer Umbra draws over them: a camera, or keyboard and mouse input. */
export type LayerFilter = 'any' | 'camera' | 'input';

export const SORT_OPTIONS: { value: SortKey; label: string }[] = [
	{ value: 'newest', label: 'Newest first' },
	{ value: 'oldest', label: 'Oldest first' },
	{ value: 'longest', label: 'Longest first' },
	{ value: 'shortest', label: 'Shortest first' },
	{ value: 'largest', label: 'Largest first' },
	{ value: 'smallest', label: 'Smallest first' },
	{ value: 'name', label: 'Name A to Z' }
];

const STORAGE_KEY = 'umbra.library.view.v1';

function restore(): { sort: SortKey; mode: ViewMode } {
	try {
		const raw = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '{}') as Partial<{ sort: SortKey; mode: ViewMode }>;
		return {
			sort: SORT_OPTIONS.some((o) => o.value === raw.sort) ? (raw.sort as SortKey) : 'newest',
			mode: raw.mode === 'list' ? 'list' : 'grid'
		};
	} catch {
		return { sort: 'newest', mode: 'grid' };
	}
}

const saved = restore();

export const view = $state({
	query: '',
	/** A game name, `__desktop__` for clips without a game, or `all`. */
	game: 'all',
	sort: saved.sort,
	mode: saved.mode,
	/** All items, only video clips, or only screenshots. */
	kind: 'all' as KindFilter,
	layer: 'any' as LayerFilter,
	/** Show clips that live in albums marked "Hide these clips from Clips". */
	showHidden: false
});

export const DESKTOP_FILTER = '__desktop__';

export function persistView() {
	try {
		localStorage.setItem(STORAGE_KEY, JSON.stringify({ sort: view.sort, mode: view.mode }));
	} catch {
		/* storage may be unavailable */
	}
}

export function resetFilters() {
	view.query = '';
	view.game = 'all';
	view.kind = 'all';
	view.layer = 'any';
}
