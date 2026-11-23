<script lang="ts">
	import { app } from '$lib/stores/app.svelte';
	import { formatBytes } from '$lib/format/format';
	import PanelSection from './PanelSection.svelte';

	const usage = $derived(app.status?.usage ?? null);

	const pct = (v: number | null | undefined) => (v === null || v === undefined ? '—' : `${Math.round(v)}%`);

	const rows = $derived([
		{ label: 'CPU', value: pct(usage?.cpuPercent), fill: usage?.cpuPercent ?? null, note: 'Share of the whole processor.' },
		{ label: 'Memory', value: usage ? formatBytes(usage.memoryBytes) : '—', fill: null, note: 'RAM in use, replay buffer included.' },
		{ label: 'GPU', value: pct(usage?.gpuPercent), fill: usage?.gpuPercent ?? null, note: '3D and compute work, which games compete for.' }
	]);
</script>

<PanelSection id="panel-usage" title="Umbra is using">
	<dl class="flex flex-col gap-2">
		{#each rows as row (row.label)}
			<div class="flex flex-col gap-1">
				<div class="flex items-baseline justify-between text-xs">
					<dt class="text-foreground">{row.label}</dt>
					<dd class="tnum text-foreground">{row.value}</dd>
				</div>
				{#if row.fill !== null}
					<span class="relative block h-1 overflow-hidden rounded-xs bg-selected" aria-hidden="true">
						<span class="absolute inset-0 origin-left bg-muted-foreground" style:transform="scaleX({Math.min(100, row.fill) / 100})" style:transition="transform 600ms linear"></span>
					</span>
				{/if}
				<dd class="text-xs text-subtle">{row.note}</dd>
			</div>
		{/each}
	</dl>
</PanelSection>
