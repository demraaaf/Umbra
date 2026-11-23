<script lang="ts">
	import SearchIcon from '@lucide/svelte/icons/search';
	import XIcon from '@lucide/svelte/icons/x';
	import LayoutGridIcon from '@lucide/svelte/icons/layout-grid';
	import ListIcon from '@lucide/svelte/icons/list';
	import ImportIcon from '@lucide/svelte/icons/import';
	import { Button } from '$lib/components/ui/button';
	import * as InputGroup from '$lib/components/ui/input-group';
	import * as Select from '$lib/components/ui/select';
	import * as ToggleGroup from '$lib/components/ui/toggle-group';
	import { Kbd } from '$lib/components/ui/kbd';
	import type { Clip } from '$lib/api/types';
	import { DESKTOP_FILTER, SORT_OPTIONS, persistView, view, type KindFilter, type LayerFilter, type ViewMode } from '$lib/stores/view.svelte';
	import { formatBytes } from '$lib/format/format';
	import { openImport } from '$lib/stores/importer.svelte';

	let {
		clips,
		shown,
		searchInput = $bindable(null)
	}: { clips: Clip[]; shown: Clip[]; searchInput?: HTMLInputElement | null } = $props();

	const games = $derived.by(() => {
		const counts = new Map<string, number>();
		let desktop = 0;
		for (const clip of clips) {
			if (clip.game) counts.set(clip.game, (counts.get(clip.game) ?? 0) + 1);
			else desktop++;
		}
		return { list: [...counts.entries()].sort((a, b) => a[0].localeCompare(b[0])), desktop };
	});

	const gameLabel = $derived(view.game === 'all' ? 'All games' : view.game === DESKTOP_FILTER ? 'Desktop' : view.game);
	const LAYERS: { value: LayerFilter; label: string }[] = [
		{ value: 'any', label: 'Any layers' },
		{ value: 'camera', label: 'Has camera' },
		{ value: 'input', label: 'Has input' }
	];
	/** Only offered once there is a clip with a camera or input to find. */
	const layered = $derived(view.layer !== 'any' || clips.some((c) => c.camera !== null || c.input));
	const layerLabel = $derived(LAYERS.find((o) => o.value === view.layer)?.label ?? 'Any layers');
	const sortLabel = $derived(SORT_OPTIONS.find((o) => o.value === view.sort)?.label ?? 'Sort');
	const totalBytes = $derived(shown.reduce((sum, clip) => sum + clip.sizeBytes, 0));

	function setMode(next: string) {
		if (next === 'grid' || next === 'list') {
			view.mode = next as ViewMode;
			persistView();
		}
	}
</script>

<div class="@container shrink-0 border-b border-line">
	<div class="flex min-h-12 flex-wrap items-center gap-2 px-4 py-2" role="toolbar" aria-label="Library filters">
		<InputGroup.Root class="w-52 min-w-40 flex-1 @3xl:max-w-60 @3xl:flex-none">
			<InputGroup.Addon>
				<SearchIcon aria-hidden="true" />
			</InputGroup.Addon>
			<InputGroup.Input
				bind:ref={searchInput}
				bind:value={view.query}
				type="search"
				placeholder="Search"
				aria-label="Search clips and screenshots"
				autocomplete="off"
				spellcheck={false}
				class="[&::-webkit-search-cancel-button]:hidden"
			/>
			<InputGroup.Addon align="inline-end">
				{#if view.query}
					<InputGroup.Button size="icon-xs" aria-label="Clear search" onclick={() => { view.query = ''; searchInput?.focus(); }}>
						<XIcon />
					</InputGroup.Button>
				{:else}
					<Kbd aria-hidden="true">/</Kbd>
				{/if}
			</InputGroup.Addon>
		</InputGroup.Root>

		<ToggleGroup.Root
			type="single"
			variant="outline"
			size="sm"
			value={view.kind}
			onValueChange={(v) => v && (view.kind = v as KindFilter)}
			aria-label="Show"
		>
			<ToggleGroup.Item value="all" class="px-2">All</ToggleGroup.Item>
			<ToggleGroup.Item value="video" class="px-2">Clips</ToggleGroup.Item>
			<ToggleGroup.Item value="recording" class="px-2">Recordings</ToggleGroup.Item>
			<ToggleGroup.Item value="image" class="px-2">Screenshots</ToggleGroup.Item>
		</ToggleGroup.Root>

		<Select.Root type="single" bind:value={view.game}>
			<Select.Trigger size="sm" class="w-36 shrink-0" aria-label="Filter by game">
				<span class="truncate">{gameLabel}</span>
			</Select.Trigger>
			<Select.Content align="start">
				<Select.Item value="all" label="All games">All games</Select.Item>
				{#each games.list as [name, count] (name)}
					<Select.Item value={name} label={name}>
						<span class="truncate">{name}</span>
						<span class="tnum ml-auto pl-3 text-subtle">{count}</span>
					</Select.Item>
				{/each}
				{#if games.desktop > 0}
					<Select.Item value={DESKTOP_FILTER} label="Desktop">
						Desktop
						<span class="tnum ml-auto pl-3 text-subtle">{games.desktop}</span>
					</Select.Item>
				{/if}
			</Select.Content>
		</Select.Root>

		{#if layered}
			<Select.Root type="single" bind:value={view.layer}>
				<Select.Trigger size="sm" class="w-32 shrink-0" aria-label="Filter by layer">
					<span class="truncate">{layerLabel}</span>
				</Select.Trigger>
				<Select.Content align="start">
					{#each LAYERS as option (option.value)}
						<Select.Item value={option.value} label={option.label}>{option.label}</Select.Item>
					{/each}
				</Select.Content>
			</Select.Root>
		{/if}

		<Select.Root type="single" bind:value={view.sort} onValueChange={persistView}>
			<Select.Trigger size="sm" class="w-36 shrink-0" aria-label="Sort">
				<span class="truncate">{sortLabel}</span>
			</Select.Trigger>
			<Select.Content align="start">
				{#each SORT_OPTIONS as option (option.value)}
					<Select.Item value={option.value} label={option.label}>{option.label}</Select.Item>
				{/each}
			</Select.Content>
		</Select.Root>

		<div class="ml-auto flex shrink-0 items-center gap-3">
			<Button variant="outline" size="sm" onclick={openImport} title="Bring in clips from other recorders or a folder">
				<ImportIcon data-icon="inline-start" />
				Import
			</Button>
			<p class="tnum hidden text-xs text-muted-foreground @4xl:block" aria-live="polite">
				{shown.length}{shown.length === clips.length ? '' : ` of ${clips.length}`} &middot; {formatBytes(totalBytes)}
			</p>
			<ToggleGroup.Root type="single" variant="outline" size="sm" value={view.mode} onValueChange={setMode} aria-label="Layout">
				<ToggleGroup.Item value="grid" aria-label="Grid" title="Grid">
					<LayoutGridIcon />
				</ToggleGroup.Item>
				<ToggleGroup.Item value="list" aria-label="List" title="List">
					<ListIcon />
				</ToggleGroup.Item>
			</ToggleGroup.Root>
		</div>
	</div>

</div>

