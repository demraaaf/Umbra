<script lang="ts">
	import PlayIcon from '@lucide/svelte/icons/play';
	import XIcon from '@lucide/svelte/icons/x';
	import type { Sequence } from '$lib/editor/sequence.svelte';
	import { Button } from '$lib/components/ui/button';
	import { clipTitle, formatDuration } from '$lib/format/format';

	/**
	 * The clips in the edit, in order, sized by how long each is in the export. Click one to
	 * edit it; drag to reorder (the first stays first); × takes one out. Clips dragged from the
	 * media pool drop in where they are let go.
	 */
	let { sequence, busy = false }: { sequence: Sequence; busy?: boolean } = $props();

	const entries = $derived(sequence.entries);
	const total = $derived(Math.max(0.001, sequence.totalSecs));

	let dragging = $state<number | null>(null);
	let dropAt = $state<number | null>(null);

	function indexAt(e: DragEvent, i: number): number {
		const el = e.currentTarget as HTMLElement;
		const rect = el.getBoundingClientRect();
		const after = e.clientX > rect.left + rect.width / 2;
		return Math.max(1, i + (after ? 1 : 0));
	}

	function onDrop(e: DragEvent) {
		e.preventDefault();
		const at = dropAt;
		dropAt = null;
		if (at === null || busy) return;
		const path = e.dataTransfer?.getData('application/x-umbra-clip');
		if (path) {
			sequence.add(path, at);
		} else if (dragging !== null) {
			sequence.move(dragging, at > dragging ? at - 1 : at);
		}
		dragging = null;
	}

	function playAll() {
		const first = entries[0];
		if (!first) return;
		sequence.playAll = true;
		sequence.select(0);
		first.session.playback.seek(first.session.inSecs);
		first.session.playback.play();
	}
</script>

<div class="flex items-center gap-2 border-t border-line px-3 py-2">
	<div class="flex min-w-0 flex-1 items-stretch gap-1.5 overflow-x-auto" role="listbox" aria-label="Clips in this edit" tabindex="-1" ondragover={(e) => e.preventDefault()} ondrop={onDrop}>
		{#each entries as entry, i (entry.clip.path)}
			{@const active = i === sequence.active}
			<div
				role="option"
				aria-selected={active}
				tabindex="0"
				draggable={i > 0 && !busy}
				class="block relative flex min-w-[124px] items-center gap-2 rounded-md border p-1 pr-6 text-left"
				class:active
				class:drop-before={dropAt === i}
				class:drop-after={dropAt === i + 1 && i === entries.length - 1}
				style:flex-grow={entry.session.outputSecs / total}
				onclick={() => sequence.select(i)}
				onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), sequence.select(i))}
				ondragstart={(e) => {
					dragging = i;
					e.dataTransfer?.setData('text/plain', String(i));
				}}
				ondragend={() => ((dragging = null), (dropAt = null))}
				ondragover={(e) => {
					e.preventDefault();
					dropAt = indexAt(e, i);
				}}
			>
				<span class="relative aspect-video h-9 shrink-0 overflow-hidden rounded-xs bg-black">
					{#if entry.clip.thumbnailUrl}<img src={entry.clip.thumbnailUrl} alt="" class="size-full object-cover" draggable="false" />{/if}
					<span class="tnum absolute top-0 left-0 rounded-br-xs bg-black/75 px-1 text-[10px] text-foreground">{i + 1}</span>
				</span>
				<span class="flex min-w-0 flex-col">
					<span class="truncate text-xs text-foreground" title={entry.clip.fileName}>{clipTitle(entry.clip)}</span>
					<span class="tnum text-[11px] text-subtle">{formatDuration(entry.session.outputSecs)}</span>
				</span>
				{#if i > 0}
					<button
						type="button"
						class="remove"
						aria-label="Take {clipTitle(entry.clip)} out of the edit"
						title="Take out of the edit"
						disabled={busy}
						onclick={(e) => {
							e.stopPropagation();
							sequence.remove(i);
						}}
					>
						<XIcon class="size-3" aria-hidden="true" />
					</button>
				{/if}
			</div>
		{/each}
	</div>
	<div class="flex shrink-0 flex-col items-end gap-1">
		<Button variant="outline" size="sm" onclick={playAll} disabled={busy} title="Play every clip in order, as the export will be">
			<PlayIcon data-icon="inline-start" />
			Play all
		</Button>
		<span class="tnum text-[11px] text-subtle">{entries.length} clips · {formatDuration(sequence.totalSecs)}</span>
	</div>
</div>

<style>
	[role='option'] {
		border-color: var(--line);
		background: var(--surface-panel);
		cursor: pointer;
		transition:
			border-color var(--dur-fast) var(--ease-out),
			background-color var(--dur-fast) var(--ease-out);
	}
	[role='option']:hover {
		border-color: var(--line-strong);
	}
	[role='option'].active {
		border-color: var(--text-muted);
		background: var(--surface-raised);
	}
	[role='option']:focus-visible {
		outline: 2px solid var(--focus);
		outline-offset: 2px;
	}
	/* Where a dragged clip will land. */
	.drop-before {
		box-shadow: -3px 0 0 0 var(--text);
	}
	.drop-after {
		box-shadow: 3px 0 0 0 var(--text);
	}
	.remove {
		position: absolute;
		top: 4px;
		right: 4px;
		display: inline-flex;
		padding: 2px;
		border-radius: 3px;
		color: var(--text-subtle);
	}
	.remove:hover:not(:disabled) {
		color: var(--text);
		background: var(--surface-on);
	}
</style>
