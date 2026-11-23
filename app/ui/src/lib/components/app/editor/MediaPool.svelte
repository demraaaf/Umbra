<script lang="ts">
	import CheckIcon from '@lucide/svelte/icons/check';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import SearchIcon from '@lucide/svelte/icons/search';
	import * as InputGroup from '$lib/components/ui/input-group';
	import { Button } from '$lib/components/ui/button';
	import type { Sequence } from '$lib/editor/sequence.svelte';
	import { app } from '$lib/stores/app.svelte';
	import { clipTitle, formatDuration, gameLabel } from '$lib/format/format';

	/**
	 * The library's clips, to put into the edit: Add places one after the clip being edited;
	 * a clip can also be dragged onto the strip of clips under the player.
	 */
	let { sequence, busy = false }: { sequence: Sequence; busy?: boolean } = $props();

	let query = $state('');

	const clips = $derived.by(() => {
		const q = query.trim().toLowerCase();
		return app.clips
			.filter((c) => c.kind === 'video')
			.filter((c) => !q || clipTitle(c).toLowerCase().includes(q) || gameLabel(c.game).toLowerCase().includes(q))
			.sort((a, b) => b.createdAt.localeCompare(a.createdAt));
	});
</script>

<div class="flex min-h-0 flex-1 flex-col">
	<div class="flex flex-col gap-2 border-b border-line p-3">
		<p class="text-xs text-muted-foreground">
			Add clips to make one video of them, one after another. Each keeps its own trim, cuts and speed.
		</p>
		<InputGroup.Root>
			<InputGroup.Addon><SearchIcon aria-hidden="true" /></InputGroup.Addon>
			<InputGroup.Input bind:value={query} placeholder="Search clips" aria-label="Search clips" autocomplete="off" spellcheck={false} />
		</InputGroup.Root>
	</div>
	<ul class="min-h-0 flex-1 overflow-y-auto p-1.5" aria-label="Clips in the library">
		{#each clips as clip (clip.path)}
			{@const added = sequence.has(clip.path)}
			<li
				class="row flex items-center gap-2.5 rounded-md p-1.5"
				draggable={!added && !busy}
				ondragstart={(e) => {
					e.dataTransfer?.setData('application/x-umbra-clip', clip.path);
					if (e.dataTransfer) e.dataTransfer.effectAllowed = 'copy';
				}}
			>
				<span class="relative aspect-video w-16 shrink-0 overflow-hidden rounded-xs bg-black">
					{#if clip.thumbnailUrl}<img src={clip.thumbnailUrl} alt="" class="size-full object-cover" draggable="false" />{/if}
				</span>
				<span class="flex min-w-0 flex-1 flex-col">
					<span class="truncate text-xs text-foreground" title={clip.fileName}>{clipTitle(clip)}</span>
					<span class="tnum truncate text-[11px] text-subtle">{gameLabel(clip.game)} · {formatDuration(clip.durationSecs)}</span>
				</span>
				{#if added}
					<span class="flex shrink-0 items-center gap-1 pr-1 text-[11px] text-muted-foreground"><CheckIcon class="size-3" aria-hidden="true" /> In</span>
				{:else}
					<Button variant="outline" size="icon-xs" class="shrink-0" disabled={busy} aria-label="Add {clipTitle(clip)}" title="Add after the clip being edited" onclick={() => sequence.add(clip.path)}>
						<PlusIcon />
					</Button>
				{/if}
			</li>
		{:else}
			<li class="p-3 text-xs text-subtle">{query ? 'No clips match.' : 'No clips yet.'}</li>
		{/each}
	</ul>
</div>

<style>
	.row {
		transition: background-color var(--dur-fast) var(--ease-out);
	}
	.row:hover {
		background: var(--surface-raised);
	}
	.row[draggable='true'] {
		cursor: grab;
	}
</style>
