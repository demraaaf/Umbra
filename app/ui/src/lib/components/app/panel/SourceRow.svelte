<script lang="ts">
	import CheckIcon from '@lucide/svelte/icons/check';
	import type { CaptureSource } from '$lib/api/types';
	import SourcePreview from '../SourcePreview.svelte';

	/** One capture source with a small live still. */
	let {
		source,
		active,
		busy,
		onpick
	}: { source: CaptureSource; active: boolean; busy: boolean; onpick: (source: CaptureSource) => void } = $props();
</script>

<button
	type="button"
	class="source flex w-full min-w-0 items-center gap-2 rounded-md p-1 text-left"
	class:active
	aria-pressed={active}
	disabled={busy}
	onclick={() => onpick(source)}
	title={active ? `${source.label} is being captured` : `Capture ${source.label}`}
>
	<SourcePreview target={source.target} empty="None" class="h-9 w-16 shrink-0 rounded-xs border border-line" />
	<span class="flex min-w-0 flex-1 flex-col">
		<span class="flex min-w-0 items-center gap-1">
			<span class="truncate text-xs font-medium text-foreground">{source.label}</span>
			{#if source.isGame}
				<span class="shrink-0 rounded-xs border border-line-strong px-1 text-xs text-muted-foreground">Game</span>
			{/if}
		</span>
		<span class="truncate text-xs text-subtle">{source.detail}</span>
	</span>
	{#if active}
		<CheckIcon class="size-4 shrink-0 text-foreground" aria-hidden="true" />
	{/if}
</button>

<style>
	.source {
		transition: background-color var(--dur-fast) var(--ease-out);
	}
	.source:hover:not(:disabled) {
		background: var(--surface-raised);
	}
	.source.active {
		background: var(--surface-selected);
	}
	.source:disabled {
		cursor: progress;
	}
</style>
