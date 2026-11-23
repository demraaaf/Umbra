<script lang="ts">
	import type { ImportActive, ImportProgress } from '$lib/api/types';
	import { Progress } from '$lib/components/ui/progress';
	import { Spinner } from '$lib/components/ui/spinner';
	import { formatBytes } from '$lib/format/format';
	import { importFraction, timeLeft } from '$lib/stores/importer.svelte';

	/**
	 * A running import: overall progress by bytes with speed and time left, then each file being
	 * worked on with what is happening to it, so a big file that takes minutes shows why.
	 */
	let { progress }: { progress: ImportProgress } = $props();

	const STAGES: Record<ImportActive['stage'], string> = {
		copying: 'Copying',
		converting: 'Converting',
		checking: 'Checking the copy',
		removing: 'Removing the original'
	};

	const fraction = $derived(importFraction(progress));
	const left = $derived(timeLeft(progress.etaSecs));
	/** 10 GB and up is worth explaining. */
	const big = $derived(progress.active.some((f) => f.bytes >= 10e9));
	const converting = $derived(progress.active.some((f) => f.stage === 'converting'));
</script>

<section class="flex flex-col gap-4" aria-labelledby="import-progress-title">
	<div class="flex flex-col gap-2">
		<p id="import-progress-title" class="flex items-center gap-2 text-sm text-foreground">
			<Spinner />
			Importing
			<span class="tnum ml-auto text-xs text-muted-foreground">{progress.done} of {progress.total} done</span>
		</p>
		<Progress value={Math.round(fraction * 1000)} max={1000} aria-label="Import progress" />
		<p class="tnum flex flex-wrap gap-x-2 text-xs text-muted-foreground" aria-live="polite">
			{#if progress.bytesTotal > 0}
				<span>{formatBytes(progress.bytesDone)} of {formatBytes(progress.bytesTotal)}</span>
			{/if}
			{#if progress.rate > 0}
				<span aria-hidden="true">·</span><span>{formatBytes(progress.rate)}/s</span>
			{/if}
			{#if left}
				<span aria-hidden="true">·</span><span>{left}</span>
			{/if}
		</p>
	</div>

	{#if progress.active.length > 0}
		<div class="flex flex-col gap-2">
			<h3 class="text-xs font-medium text-muted-foreground">Now</h3>
			<ul class="flex flex-col divide-y divide-line rounded-md border border-line-control">
				{#each progress.active as file (file.name)}
					<li class="flex flex-col gap-1.5 px-3 py-2">
						<span class="flex min-w-0 items-baseline gap-3">
							<span class="truncate text-sm text-foreground" title={file.name}>{file.name}</span>
							<span class="tnum ml-auto shrink-0 text-xs text-subtle">{formatBytes(file.bytes)}</span>
						</span>
						<span class="flex items-center gap-3">
							<span class="w-36 shrink-0 text-xs text-muted-foreground">{STAGES[file.stage]}</span>
							<span class="bar" aria-hidden="true"><span style:transform="scaleX({file.fraction})"></span></span>
							<span class="tnum w-9 shrink-0 text-right text-xs text-muted-foreground">
								{file.stage === 'removing' ? '' : `${Math.floor(file.fraction * 100)}%`}
							</span>
						</span>
					</li>
				{/each}
			</ul>
		</div>
	{/if}

	{#if big || converting}
		<p class="text-xs text-subtle">
			{#if big}
				Big files take a while: each is read to the end to copy it{progress.moving ? ', and the copy is read once more to check it before the original goes' : ''}.
			{/if}
			{#if converting}
				Converting re-encodes a video MP4 cannot hold as it is, which is much slower than copying.
			{/if}
		</p>
	{/if}
	<p class="text-xs text-muted-foreground">You can close this. The import carries on, its progress stays at the bottom of the sidebar, and it says when it is done.</p>
</section>

<style>
	.bar {
		position: relative;
		height: 4px;
		flex: 1;
		overflow: hidden;
		border-radius: 2px;
		background: var(--surface-raised);
	}
	.bar > span {
		position: absolute;
		inset: 0;
		transform-origin: left;
		background: var(--text-muted);
		transition: transform 480ms linear;
	}
	@media (prefers-reduced-motion: reduce) {
		.bar > span {
			transition: none;
		}
	}
</style>
