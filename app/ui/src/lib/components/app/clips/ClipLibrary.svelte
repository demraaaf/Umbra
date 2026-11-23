<script lang="ts">
	import { onMount } from 'svelte';
	import type { Clip, FileRecordingStatus, ProcessingItem } from '$lib/api/types';
	import { noteOpenClick, openClip } from '$lib/stores/app.svelte';
	import {
		clearSelection,
		isSelected,
		keepOnly,
		selectAll,
		selectRange,
		selection,
		setSelected,
		toggleSelected
	} from '$lib/stores/selection.svelte';
	import type { ViewMode } from '$lib/stores/view.svelte';
	import ClipCard from './ClipCard.svelte';
	import LiveRecordingCard from './LiveRecordingCard.svelte';
	import ProcessingCard from './ProcessingCard.svelte';
	import SavingClipCard from './SavingClipCard.svelte';
	import type { SavingClip } from '$lib/stores/app.svelte';
	import ClipTable from './ClipTable.svelte';
	import ClipContextMenu from './ClipContextMenu.svelte';
	import { byDay } from '$lib/format/days';

	/**
	 * The clip grid or list with selection:
	 *   click opens the player (a double-click the editor), Ctrl+click toggles, Shift+click
	 *   selects a range, the middle mouse button toggles one clip and, held down, "paints"
	 *   the selection over every clip it passes.
	 *   Ctrl+A selects everything shown, Esc clears.
	 */
	let {
		clips,
		mode,
		live = null,
		processing = [],
		saving = [],
		grouped = false
	}: {
		clips: Clip[];
		mode: ViewMode;
		/** Under a heading per day (when sorted by date). */
		grouped?: boolean;
		/** A recording in progress, shown first. */
		live?: FileRecordingStatus | null;
		/** Recordings still being finished, shown after the live one. */
		processing?: ProcessingItem[];
		/** Clips being saved, shown first. */
		saving?: SavingClip[];
	} = $props();

	let container = $state<HTMLDivElement | null>(null);
	let paint: { on: boolean } | null = null;

	const ordered = $derived(clips.map((c) => c.path));
	const selecting = $derived(selection.paths.length > 0);

	// A filter change can hide selected clips; drop them so actions only touch what is visible.
	$effect(() => {
		keepOnly(ordered);
	});

	function activate(clip: Clip, event: MouseEvent) {
		if (event.ctrlKey || event.metaKey) toggleSelected(clip.path);
		else if (event.shiftKey) selectRange(ordered, clip.path);
		else {
			// Opens at once; a second click on the player turns it into a double-click (editor).
			if (event.detail > 0) noteOpenClick(clip.path, event);
			openClip(clip.path, ordered);
		}
	}

	function pathAt(x: number, y: number): string | null {
		const el = document.elementFromPoint(x, y)?.closest<HTMLElement>('[data-clip-path]');
		return el?.dataset.clipPath ?? null;
	}

	function onmousedown(e: MouseEvent) {
		if (e.button !== 1) return;
		// Stops Chromium's middle-click autoscroll; selection takes the gesture instead.
		e.preventDefault();
		const path = pathAt(e.clientX, e.clientY);
		if (!path) return;
		const on = !isSelected(path);
		setSelected(path, on);
		selection.anchor = path;
		paint = { on };
	}

	function onmousemove(e: MouseEvent) {
		if (!paint) return;
		if ((e.buttons & 4) === 0) {
			paint = null;
			return;
		}
		const path = pathAt(e.clientX, e.clientY);
		if (path) setSelected(path, paint.on);
	}

	onMount(() => {
		const endPaint = (e: MouseEvent) => {
			if (e.button === 1) paint = null;
		};
		const onKey = (e: KeyboardEvent) => {
			if (e.defaultPrevented) return;
			const target = e.target as HTMLElement | null;
			if (target?.closest('input, textarea, select, [contenteditable="true"], [role="dialog"], [role="alertdialog"], [role="menu"]')) return;
			if ((e.ctrlKey || e.metaKey) && !e.altKey && e.code === 'KeyA') {
				e.preventDefault();
				selectAll(ordered);
			} else if (e.key === 'Escape' && selection.paths.length > 0) {
				e.preventDefault();
				clearSelection();
			}
		};
		window.addEventListener('mouseup', endPaint);
		window.addEventListener('keydown', onKey);
		return () => {
			window.removeEventListener('mouseup', endPaint);
			window.removeEventListener('keydown', onKey);
		};
	});

	/** Arrow keys move between cards the way they would in a file browser. */
	function onkeydown(e: KeyboardEvent) {
		if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'].includes(e.key)) return;
		const cards = [...(container?.querySelectorAll<HTMLElement>('[data-clip-card]') ?? [])];
		const index = cards.indexOf(document.activeElement as HTMLElement);
		if (index < 0) return;
		let next = index;
		if (e.key === 'Home') next = 0;
		else if (e.key === 'End') next = cards.length - 1;
		else if (e.key === 'ArrowLeft') next = index - 1;
		else if (e.key === 'ArrowRight') next = index + 1;
		else if (mode === 'list') next = index + (e.key === 'ArrowDown' ? 1 : -1);
		else {
			const from = cards[index].getBoundingClientRect();
			const down = e.key === 'ArrowDown';
			let best = -1;
			let bestScore = Infinity;
			cards.forEach((card, i) => {
				if (i === index) return;
				const r = card.getBoundingClientRect();
				const rowDelta = down ? r.top - from.top : from.top - r.top;
				if (rowDelta < 8) return;
				const score = rowDelta * 4 + Math.abs(r.left - from.left);
				if (score < bestScore) {
					bestScore = score;
					best = i;
				}
			});
			if (best >= 0) next = best;
		}
		next = Math.max(0, Math.min(cards.length - 1, next));
		if (next !== index) {
			e.preventDefault();
			cards[next].focus();
			cards[next].scrollIntoView({ block: 'nearest' });
		}
	}
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
	bind:this={container}
	class="select-none"
	{onmousedown}
	{onmousemove}
	onauxclick={(e) => e.preventDefault()}
	{onkeydown}
>
	{#if mode === 'grid'}
		<ul class="grid grid-cols-[repeat(auto-fill,minmax(200px,1fr))] gap-x-4 gap-y-6 p-4" aria-label="Clips">
			{#if live}
				<li class="min-w-0"><LiveRecordingCard file={live} /></li>
			{/if}
			{#each saving as item (item.id)}
				<li class="min-w-0"><SavingClipCard {item} /></li>
			{/each}
			{#each processing as item (item.path)}
				<li class="min-w-0"><ProcessingCard {item} /></li>
			{/each}
			{#each grouped ? byDay(clips) : [{ key: '', label: '', items: clips }] as group (group.key)}
				{#if grouped}
					<li class="col-span-full -mb-2 flex items-baseline gap-2 border-b border-line pb-1.5 first:pt-0" role="presentation">
						<h3 class="text-sm font-medium text-foreground">{group.label}</h3>
						<span class="tnum text-xs text-subtle">{group.items.length}</span>
					</li>
				{/if}
				{#each group.items as clip (clip.path)}
					<ClipContextMenu {clip} list={ordered}>
						{#snippet trigger(props)}
							<li class="min-w-0" {...props}>
								<ClipCard
									{clip}
									selected={isSelected(clip.path)}
									{selecting}
									onactivate={activate}
									ontoggle={(c) => toggleSelected(c.path)}
								/>
							</li>
						{/snippet}
					</ClipContextMenu>
				{/each}
			{/each}
		</ul>
	{:else}
		<div class="px-2 pb-4">
			{#if live}
				<div class="max-w-xs px-2 pt-3"><LiveRecordingCard file={live} /></div>
			{/if}
			{#if processing.length > 0 || saving.length > 0}
				<div class="grid grid-cols-[repeat(auto-fill,minmax(200px,20rem))] gap-4 px-2 pt-3">
					{#each saving as item (item.id)}
						<SavingClipCard {item} />
					{/each}
					{#each processing as item (item.path)}
						<ProcessingCard {item} />
					{/each}
				</div>
			{/if}
			<ClipTable {clips} {grouped} onactivate={activate} ontoggle={(c) => toggleSelected(c.path)} />
		</div>
	{/if}
</div>
