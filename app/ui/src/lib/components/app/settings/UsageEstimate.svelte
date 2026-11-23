<script lang="ts">
	import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
	import { api } from '$lib/api';
	import type { Settings, UsageEstimate } from '$lib/api/types';
	import { EXPLAIN } from '$lib/format/explain';
	import { formatBytes } from '$lib/format/format';
	import Term from '../Term.svelte';
	import CostBreakdownDialog from './CostBreakdownDialog.svelte';

	/**
	 * What the settings on this page would cost, worked out by the backend without applying them.
	 * Re-estimated 300 ms after the last change, and kept in view above the settings.
	 */
	let { draft }: { draft: Settings } = $props();

	let estimate = $state<UsageEstimate | null>(null);
	let failed = $state(false);
	let expanded = $state(false);
	let breakdownOpen = $state(false);

	const LEVELS: Record<UsageEstimate['level'], string> = {
		light: 'Light',
		moderate: 'Moderate',
		heavy: 'Heavy',
		'too-heavy': 'Too heavy'
	};

	$effect(() => {
		const snapshot = JSON.stringify(draft);
		let cancelled = false;
		const timer = setTimeout(async () => {
			try {
				const next = await api.estimateUsage(JSON.parse(snapshot) as Settings);
				if (!cancelled) {
					estimate = next;
					failed = false;
				}
			} catch {
				if (!cancelled) failed = true;
			}
		}, 300);
		return () => {
			cancelled = true;
			clearTimeout(timer);
		};
	});

	const tooHeavy = $derived(estimate?.level === 'too-heavy');
	const loadPercent = $derived(estimate ? Math.round(estimate.encoderLoad * 100) : 0);
</script>

<section class="sticky top-0 z-10 -mx-6 border-b border-line bg-canvas px-6 py-3" aria-labelledby="estimate-title" aria-live="polite">
	<div class="flex items-center gap-2">
		<h2 id="estimate-title" class="text-xs font-medium text-muted-foreground">Estimate at these settings</h2>
		{#if estimate}
			{#if estimate.breakdown.length > 0}
				<button type="button" class="more ml-auto text-xs" onclick={() => (breakdownOpen = true)}>What costs what</button>
			{/if}
			<span class="level {estimate.breakdown.length > 0 ? '' : 'ml-auto'}" class:danger={tooHeavy}>{LEVELS[estimate.level]}</span>
		{/if}
	</div>

	{#if failed && !estimate}
		<p class="mt-2 text-xs text-muted-foreground">The estimate is not available right now.</p>
	{:else if !estimate}
		<p class="mt-2 text-xs text-subtle" role="status">Working it out</p>
	{:else}
		<dl class="mt-2 grid grid-cols-2 gap-x-6 gap-y-2 text-xs @2xl:grid-cols-4">
			<div class="flex flex-col">
				<dt class="text-muted-foreground"><Term label={draft.clip.bufferOnDisk ? "Buffer on disk" : "Buffer memory"} explain={EXPLAIN.buffer} /></dt>
				<dd class="tnum text-sm text-foreground">{formatBytes(estimate.bufferBytes)}</dd>
			</div>
			<div class="flex flex-col">
				<dt class="text-muted-foreground"><Term label="Clip size" explain={EXPLAIN.clipSize} /></dt>
				<dd class="tnum text-sm text-foreground">{formatBytes(estimate.clipBytes)}</dd>
			</div>
			<div class="flex flex-col">
				<dt class="text-muted-foreground"><Term label="CPU" explain={EXPLAIN.cpu} /></dt>
				<dd class="tnum text-sm text-foreground">{Math.round(estimate.cpuPercent)}%</dd>
			</div>
			<div class="flex flex-col gap-1">
				<dt class="text-muted-foreground"><Term label="Encoder load" explain={EXPLAIN.encoderLoad} /></dt>
				<dd class="flex items-center gap-2">
					<span
						class="relative h-1 flex-1 overflow-hidden rounded-xs bg-selected"
						role="meter"
						aria-label="Encoder load"
						aria-valuemin={0}
						aria-valuemax={100}
						aria-valuenow={Math.min(100, loadPercent)}
						aria-valuetext="{loadPercent}%{estimate.encoderLoad > 1 ? ', cannot keep up' : ''}"
					>
						<span class="bar absolute inset-0 origin-left" class:danger={tooHeavy} style:transform="scaleX({Math.min(100, loadPercent) / 100})"></span>
					</span>
					<span class="tnum w-10 text-right text-sm {tooHeavy ? 'text-danger' : 'text-foreground'}">{loadPercent}%</span>
				</dd>
			</div>
		</dl>

		{#if estimate.notes.length > 0}
			<div class="mt-2 flex items-start gap-2 text-xs">
				{#if expanded}
					<ul class="flex flex-1 flex-col gap-1 text-muted-foreground">
						{#each estimate.notes as note (note)}
							<li>{note}</li>
						{/each}
					</ul>
				{:else}
					<p class="min-w-0 flex-1 truncate text-muted-foreground" title={estimate.notes.join(' ')}>{estimate.notes[0]}</p>
				{/if}
				{#if estimate.notes.length > 1 || estimate.notes[0].length > 80}
					<button type="button" class="more" aria-expanded={expanded} onclick={() => (expanded = !expanded)}>
						{expanded ? 'Less' : estimate.notes.length > 1 ? `${estimate.notes.length} notes` : 'More'}
						<ChevronDownIcon class="size-3 {expanded ? 'rotate-180' : ''}" aria-hidden="true" />
					</button>
				{/if}
			</div>
		{/if}
		<p class="sr-only">This is an estimate. Real use depends on the game and what is on screen.</p>
		<CostBreakdownDialog bind:open={breakdownOpen} costs={estimate.breakdown} />
	{/if}
</section>

<style>
	.level {
		border: 1px solid var(--line-strong);
		border-radius: 2px;
		padding: 0 4px;
		font-size: 12px;
		line-height: 16px;
		color: var(--text);
	}
	.level.danger {
		border-color: var(--danger);
		color: var(--danger);
	}
	.bar {
		background: var(--text-muted);
		transition: transform var(--dur-slow) var(--ease-out);
	}
	.bar.danger {
		background: var(--danger);
	}
	.more {
		display: inline-flex;
		flex-shrink: 0;
		align-items: center;
		gap: 4px;
		border-radius: 2px;
		color: var(--text);
	}
</style>
