<script lang="ts">
	import { onMount } from 'svelte';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import { api } from '$lib/api';
	import type { CaptureSource, CaptureTarget } from '$lib/api/types';
	import { Button } from '$lib/components/ui/button';
	import { app, setCaptureTarget } from '$lib/stores/app.svelte';
	import IconButton from '../IconButton.svelte';
	import PanelSection from './PanelSection.svelte';
	import SourceRow from './SourceRow.svelte';

	let sources = $state<CaptureSource[]>([]);
	let loading = $state(true);
	let showAll = $state(false);
	let switching = $state(false);

	async function load() {
		loading = true;
		try {
			sources = await api.listCaptureSources();
		} catch {
			sources = [];
		} finally {
			loading = false;
		}
	}

	onMount(() => {
		void load();
	});

	function same(a: CaptureTarget, b: CaptureTarget, firstMonitor: string | undefined): boolean {
		if (a.kind === 'monitor' && b.kind === 'monitor') {
			// An empty device means the primary monitor, which the switcher lists first.
			return (a.device || firstMonitor) === (b.device || firstMonitor);
		}
		if (a.kind === 'window' && b.kind === 'window') return a.exe === b.exe && a.title === b.title;
		return false;
	}

	const firstMonitor = $derived(
		sources.find((s) => s.target.kind === 'monitor')?.target as { kind: 'monitor'; device: string } | undefined
	);
	const current = $derived(app.settings?.capture);

	const isActive = (s: CaptureSource) => (current ? same(s.target, current, firstMonitor?.device) : false);

	// Monitors, games and whatever is active are always listed; other windows on request.
	const primary = $derived(sources.filter((s) => s.target.kind === 'monitor' || s.isGame || isActive(s)));
	const others = $derived(sources.filter((s) => !primary.includes(s)));

	async function pick(source: CaptureSource) {
		if (isActive(source) || switching) return;
		switching = true;
		await setCaptureTarget(source.target);
		switching = false;
	}

	const key = (s: CaptureSource) =>
		s.target.kind === 'monitor' ? `m:${s.target.device}` : `w:${s.target.exe}:${s.target.title}`;
</script>

<PanelSection id="panel-source" title="Capture source">
	{#snippet action()}
		<IconButton label="Refresh the list" icon={RefreshCwIcon} size="icon-xs" onclick={load} disabled={loading} />
	{/snippet}
	{#if loading && sources.length === 0}
		<p class="text-xs text-subtle" role="status">Looking for screens and windows</p>
	{:else if sources.length === 0}
		<p class="text-xs text-subtle">Nothing to capture was found.</p>
	{:else}
		<div class="flex flex-col gap-1" role="group" aria-labelledby="panel-source">
			{#each primary as source (key(source))}
				<SourceRow {source} active={isActive(source)} busy={switching} onpick={pick} />
			{/each}
			{#if showAll}
				{#each others as source (key(source))}
					<SourceRow {source} active={isActive(source)} busy={switching} onpick={pick} />
				{/each}
			{/if}
		</div>
		{#if others.length > 0}
			<Button variant="ghost" size="xs" class="self-start" onclick={() => (showAll = !showAll)}>
				{showAll ? 'Show fewer windows' : `Show ${others.length} more ${others.length === 1 ? 'window' : 'windows'}`}
			</Button>
		{/if}
	{/if}
</PanelSection>
