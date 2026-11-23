<script lang="ts" module>
	/** Where the library was scrolled to, kept while the screen is away (watching a clip). */
	let savedScroll = 0;
</script>

<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { Button } from '$lib/components/ui/button';
	import * as Empty from '$lib/components/ui/empty';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import ClipLibrary from '$lib/components/app/clips/ClipLibrary.svelte';
	import ClipsToolbar from '$lib/components/app/clips/ClipsToolbar.svelte';
	import HiddenNote from '$lib/components/app/clips/HiddenNote.svelte';
	import SelectionBar from '$lib/components/app/clips/SelectionBar.svelte';
	import { app, hiddenPaths, refreshClips } from '$lib/stores/app.svelte';
	import { selection } from '$lib/stores/selection.svelte';
	import { DESKTOP_FILTER, resetFilters, view } from '$lib/stores/view.svelte';
	import { sortClips } from '$lib/stores/sort';
	import { comboLabels } from '$lib/format/hotkey';
	import { recordingKind } from '$lib/format/format';

	let searchInput = $state<HTMLInputElement | null>(null);
	let scroller = $state<HTMLDivElement | null>(null);

	// Back from a clip: the library is where it was, not at the top.
	onMount(() => {
		const to = savedScroll;
		if (to <= 0) return;
		void tick().then(() => {
			if (!scroller) return;
			scroller.scrollTop = to;
			// Thumbnails can still be sizing the grid; once more on the next frame.
			requestAnimationFrame(() => {
				if (scroller && scroller.scrollTop < to) scroller.scrollTop = to;
			});
		});
	});

	const hidden = $derived(hiddenPaths());
	/** Everything the Clips screen may show: clips in hidden albums are left out unless revealed. */
	/** Files still being worked on show as in progress, not as clips. */
	const busy = $derived(new Set(app.processing.map((p) => p.path)));
	const pool = $derived(
		(view.showHidden ? app.clips : app.clips.filter((c) => !hidden.has(c.path))).filter((c) => !busy.has(c.path))
	);
	const hiddenCount = $derived(app.clips.length - app.clips.filter((c) => !hidden.has(c.path)).length);

	const shown = $derived.by(() => {
		const query = view.query.trim().toLowerCase();
		return sortClips(
			pool.filter((clip) => {
				if (view.kind === 'image' && clip.kind !== 'image') return false;
				if (view.kind === 'video' && (clip.kind !== 'video' || recordingKind(clip))) return false;
				if (view.kind === 'recording' && !recordingKind(clip)) return false;
				if (view.game === DESKTOP_FILTER && clip.game) return false;
				if (view.game !== 'all' && view.game !== DESKTOP_FILTER && clip.game !== view.game) return false;
				if (view.layer === 'camera' && !clip.camera) return false;
				if (view.layer === 'input' && !clip.input) return false;
				if (!query) return true;
				return `${clip.fileName} ${clip.game ?? 'desktop'}`.toLowerCase().includes(query);
			}),
			view.sort
		);
	});

	/** The recording in progress, shown first unless the filters leave recordings out. */
	const live = $derived(
		app.status?.file && (view.kind === 'all' || view.kind === 'recording') && !view.query.trim() ? app.status.file : null
	);
	/** Recordings being finished, shown first unless the filters leave recordings out. */
	const processing = $derived(
		(view.kind === 'all' || view.kind === 'recording') && !view.query.trim() && view.layer === 'any'
			? app.processing.filter((p) => view.game === 'all' || p.game === view.game)
			: []
	);
	/** Clips on their way, unless the filters leave clips out. */
	const saving = $derived(
		(view.kind === 'all' || view.kind === 'video') && !view.query.trim() ? app.savingClips : []
	);
	const filtering = $derived(view.query.trim() !== '' || view.game !== 'all' || view.kind !== 'all' || view.layer !== 'any');
	const hotkey = $derived(app.settings ? comboLabels(app.settings.clip.hotkey).join(' + ') : '');

	const emptyMessage = $derived.by(() => {
		const query = view.query.trim();
		const what =
			view.kind === 'image' ? 'screenshots' : view.kind === 'video' ? 'clips' : view.kind === 'recording' ? 'recordings' : 'items';
		const game = view.game === DESKTOP_FILTER ? 'Desktop' : view.game;
		if (query && view.game !== 'all') return `Nothing matches “${query}” in ${game}.`;
		if (query) return `Nothing matches “${query}”.`;
		if (view.game !== 'all') return `There are no ${what} from ${game}.`;
		return `There are no ${what} yet.`;
	});

	onMount(() => {
		const onKey = (e: KeyboardEvent) => {
			const target = e.target as HTMLElement | null;
			const typing = target?.closest('input, textarea, select, [contenteditable="true"]');
			if (e.defaultPrevented) return;
			if ((e.key === '/' && !typing) || ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'f')) {
				e.preventDefault();
				searchInput?.focus();
				searchInput?.select();
			}
		};
		window.addEventListener('keydown', onKey);
		return () => window.removeEventListener('keydown', onKey);
	});
</script>

<section class="flex min-h-0 flex-1 flex-col" aria-label="Clips">
	{#if selection.paths.length > 0}
		<SelectionBar />
	{:else}
		<ClipsToolbar clips={pool} {shown} bind:searchInput />
	{/if}
	<HiddenNote count={hiddenCount} />

	<div class="min-h-0 flex-1 overflow-y-auto" bind:this={scroller} onscroll={() => (savedScroll = scroller?.scrollTop ?? 0)}>
		{#if app.clipsState === 'loading'}
			<div class="grid grid-cols-[repeat(auto-fill,minmax(200px,1fr))] gap-4 p-4" aria-busy="true" aria-label="Loading clips">
				{#each { length: 12 } as _, i (i)}
					<div class="flex flex-col gap-2">
						<Skeleton class="aspect-video w-full rounded-md" />
						<Skeleton class="h-4 w-3/4" />
						<Skeleton class="h-4 w-1/2" />
					</div>
				{/each}
			</div>
		{:else if app.clipsState === 'error'}
			<Empty.Root class="h-full border-0">
				<Empty.Header>
					<Empty.Title>Clips could not be loaded</Empty.Title>
					<Empty.Description class="selectable">{app.clipsError}</Empty.Description>
				</Empty.Header>
				<Empty.Content>
					<Button variant="outline" onclick={refreshClips}>Try again</Button>
				</Empty.Content>
			</Empty.Root>
		{:else if app.clips.length === 0 && !live && processing.length === 0 && saving.length === 0}
			<Empty.Root class="h-full border-0">
				<Empty.Header>
					<Empty.Title>No clips yet</Empty.Title>
					<Empty.Description>
						{#if app.status?.state === 'recording'}
							Recording is on. Press {hotkey || 'the hotkey'} to save the last {app.settings?.clip.lengthSecs ?? 60} seconds,
							or use Save clip in the recording panel.
						{:else}
							Start recording, then press {hotkey || 'the hotkey'} to save the last {app.settings?.clip.lengthSecs ?? 60} seconds.
						{/if}
					</Empty.Description>
				</Empty.Header>
			</Empty.Root>
		{:else if shown.length === 0 && !live && processing.length === 0 && saving.length === 0}
			<Empty.Root class="h-full border-0">
				<Empty.Header>
					<Empty.Title>Nothing to show</Empty.Title>
					<Empty.Description>{emptyMessage}</Empty.Description>
				</Empty.Header>
				{#if filtering}
					<Empty.Content>
						<Button variant="outline" onclick={resetFilters}>Clear filters</Button>
					</Empty.Content>
				{/if}
			</Empty.Root>
		{:else}
			<ClipLibrary clips={shown} mode={view.mode} grouped={view.sort === 'newest' || view.sort === 'oldest'} {live} {processing} {saving} />
		{/if}
	</div>
</section>
