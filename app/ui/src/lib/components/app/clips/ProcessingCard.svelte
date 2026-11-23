<script lang="ts">
	import type { ProcessingItem } from '$lib/api/types';
	import XIcon from '@lucide/svelte/icons/x';
	import { Spinner } from '$lib/components/ui/spinner';
	import IconButton from '$lib/components/app/IconButton.svelte';
	import { stopBurnIn, stopCompressing } from '$lib/stores/app.svelte';
	import { formatBytes } from '$lib/format/format';
	import { timeLeft } from '$lib/stores/importer.svelte';

	/**
	 * A recording that stopped and is being finished: rewritten as an ordinary MP4 so it opens
	 * at once. A placeholder in the shape of a clip card, with how far it is, so a long session
	 * that takes minutes does not look lost.
	 */
	let { item }: { item: ProcessingItem } = $props();

	const session = $derived(item.kind === 'session');
	const shrinking = $derived(item.task === 'shrink');
	const burning = $derived(item.task === 'burn');
	/** Compressing and burning in work on a clip, not a recording being finished. */
	const onClip = $derived(shrinking || burning);
	const fraction = $derived(item.bytesTotal > 0 ? Math.min(1, item.bytesDone / item.bytesTotal) : 0);
	const percent = $derived(Math.floor(fraction * 100));
	const left = $derived(timeLeft(item.etaSecs));
	/** Read to the end: the file's index is written last, which takes a moment. */
	const closing = $derived(fraction >= 0.995);
</script>

<div
	class="relative flex min-w-0 flex-col overflow-hidden rounded-md border border-line bg-chrome"
	role="group"
	aria-label="{burning ? 'Being burned in' : shrinking ? 'Being compressed' : session ? 'Session being finished' : 'Recording being finished'}, {percent}%"
	title={burning
		? 'Drawing its layers into the picture, encoded again at about its own quality. The audio is copied as it is.'
		: shrinking
		? 'Encoding the recording again as HEVC at a lower bitrate; the original goes once this one checks out.'
		: 'Rewriting the recording as an ordinary MP4 so it opens and seeks at once. Nothing is re-encoded; it reads the whole file once.'}
>
	<span class="shimmer relative block aspect-video w-full overflow-hidden" aria-hidden="true">
		<span class="absolute top-2 left-2 flex items-center gap-1.5 rounded-xs bg-black/80 px-1.5 py-0.5 text-xs font-medium text-foreground">
			<Spinner class="size-3" />
			{burning ? 'Burning in' : shrinking ? 'Compressing' : 'Finishing'}
		</span>
		<span class="tnum absolute right-1 bottom-2 rounded-xs bg-black/80 px-1 text-xs font-medium text-foreground">{percent}%</span>
		<span class="bar absolute inset-x-0 bottom-0"><span style:transform="scaleX({fraction})"></span></span>
	</span>
	<span class="flex min-w-0 flex-col gap-0.5 border-t border-line px-2.5 pt-2 pb-2.5">
		<span class="truncate text-sm font-medium text-foreground">
			{onClip ? item.fileName.replace(/[.]mp4$/i, '') : session ? `${item.game} session` : `Recording ${item.game}`}
		</span>
		<span class="tnum truncate text-xs text-muted-foreground" aria-live="polite">
			{#if onClip}
				{percent}%{left ? ` · ${left}` : ''}
			{:else if closing}
				Writing the index · {formatBytes(item.bytesTotal)}
			{:else}
				{formatBytes(item.bytesDone)} of {formatBytes(item.bytesTotal)}{left ? ` · ${left}` : ''}
			{/if}
		</span>
		<span class="truncate text-xs text-subtle">
			{burning ? 'Drawing its layers in' : shrinking ? 'Encoding it again, smaller' : 'Making it quick to open'}
		</span>
	</span>
	{#if burning}
		<!-- Stopping leaves the original as it is and writes nothing. -->
		<IconButton
			label="Stop burning in"
			icon={XIcon}
			size="icon-xs"
			variant="secondary"
			class="absolute top-1.5 right-1.5"
			onclick={() => stopBurnIn([item.path])}
		/>
	{:else if shrinking}
		<!-- Stopping leaves the original as it is, and the automatic pass leaves it alone. -->
		<IconButton
			label="Stop compressing"
			icon={XIcon}
			size="icon-xs"
			variant="secondary"
			class="absolute top-1.5 right-1.5"
			onclick={() => stopCompressing([item.path])}
		/>
	{/if}
</div>

<style>
	/* A skeleton for the thumbnail that does not exist yet: a slow sheen over the surface. */
	.shimmer {
		background: var(--surface-raised);
	}
	.shimmer::before {
		content: '';
		position: absolute;
		inset: 0;
		background: linear-gradient(100deg, transparent 30%, color-mix(in srgb, var(--text) 6%, transparent) 50%, transparent 70%);
		transform: translateX(-100%);
		animation: sheen 1.8s ease-in-out infinite;
	}
	@keyframes sheen {
		to {
			transform: translateX(100%);
		}
	}
	.bar {
		height: 3px;
		background: color-mix(in srgb, var(--text) 10%, transparent);
	}
	.bar > span {
		position: absolute;
		inset: 0;
		transform-origin: left;
		background: var(--text);
		transition: transform 900ms linear;
	}
	@media (prefers-reduced-motion: reduce) {
		.shimmer::before {
			animation: none;
		}
		.bar > span {
			transition: none;
		}
	}
</style>
