<script lang="ts">
	import FilmIcon from '@lucide/svelte/icons/film';
	import LibraryIcon from '@lucide/svelte/icons/library';
	import ScissorsIcon from '@lucide/svelte/icons/scissors';
	import SlidersHorizontalIcon from '@lucide/svelte/icons/sliders-horizontal';
	import type { Component } from 'svelte';
	import * as Tooltip from '$lib/components/ui/tooltip';
	import { app, draftIsDirty, go, openInEditor, type Screen } from '$lib/stores/app.svelte';
	import { layout } from '$lib/stores/layout.svelte';
	import { SETTINGS_GROUPS, openSettingsPage, settingsNav } from '$lib/stores/settingsNav.svelte';
	import { fade, slide } from 'svelte/transition';
	import { onMount } from 'svelte';
	import { importFraction, importer, initImporter, openImport, timeLeft } from '$lib/stores/importer.svelte';
	import { exports, exportSecsLeft } from '$lib/stores/exports.svelte';
	import SidebarProgress from './SidebarProgress.svelte';

	interface NavItem {
		screen: Screen;
		label: string;
		icon: Component<{ class?: string }>;
		/** Other screens that count as this item being current. */
		also?: Screen[];
	}

	const items: NavItem[] = [
		{ screen: 'clips', label: 'Clips', icon: FilmIcon, also: ['viewer', 'player'] },
		{ screen: 'albums', label: 'Albums', icon: LibraryIcon, also: ['album'] },
		{ screen: 'editor', label: 'Editor', icon: ScissorsIcon },
		{ screen: 'settings', label: 'Settings', icon: SlidersHorizontalIcon }
	];

	const dirty = $derived(draftIsDirty());
	const searching = $derived(settingsNav.query.trim() !== '');
	const reduceMotion = typeof matchMedia !== 'undefined' && matchMedia('(prefers-reduced-motion: reduce)').matches;
	const collapsed = $derived(layout.leftCollapsed);

	// An import keeps going with its dialog closed; its progress shows at the bottom.
	onMount(() => void initImporter());
	const importing = $derived(importer.running && importer.progress ? importer.progress : null);
	const importPercent = $derived(importing ? Math.floor(importFraction(importing) * 100) : 0);
	const importLeft = $derived(importing ? timeLeft(importing.etaSecs, true) : null);
	// Time left for exports is worked out from their pace, so it ticks on its own.
	let now = $state(performance.now());
	$effect(() => {
		if (exports.running.length === 0) return;
		const timer = setInterval(() => (now = performance.now()), 1000);
		return () => clearInterval(timer);
	});
</script>

<nav class="sidebar flex shrink-0 flex-col border-r border-line bg-chrome" class:collapsed aria-label="Main">
	<ul class="flex flex-col gap-1 p-2">
		{#each items as item (item.screen)}
			{@const disabled = item.screen === 'editor' && !app.editorPath}
			{@const active = app.screen === item.screen || (item.also?.includes(app.screen) ?? false)}
			<li>
				<Tooltip.Root disabled={!collapsed && !disabled}>
					<Tooltip.Trigger>
						{#snippet child({ props })}
							<button
								{...props}
								type="button"
								class="nav-item"
								class:active
								aria-current={active ? 'page' : undefined}
								aria-disabled={disabled}
								aria-label={item.label}
								onclick={() => !disabled && go(item.screen)}
							>
								<item.icon class="size-4 shrink-0" />
								<span class="label truncate">{item.label}</span>
								{#if item.screen === 'clips' && app.clipsState === 'ready'}
									<span class="label tnum ml-auto text-xs text-subtle" aria-hidden="true">{app.clips.length}</span>
								{:else if item.screen === 'albums' && app.albumsState === 'ready' && app.albums.length > 0}
									<span class="label tnum ml-auto text-xs text-subtle" aria-hidden="true">{app.albums.length}</span>
								{:else if item.screen === 'settings' && dirty}
									<span class="label ml-auto size-2 rounded-full bg-foreground" role="img" aria-label="Unapplied changes"></span>
								{/if}
							</button>
						{/snippet}
					</Tooltip.Trigger>
					<Tooltip.Content side="right">{disabled ? 'Open a clip to edit it' : item.label}</Tooltip.Content>
				</Tooltip.Root>
				{#if item.screen === 'settings' && app.screen === 'settings' && !collapsed}
					<ul class="pages mt-1 flex flex-col pl-6" aria-label="Settings pages" transition:slide={{ duration: reduceMotion ? 0 : 160 }}>
						{#each SETTINGS_GROUPS as group, g (group.label)}
							<li class="flex flex-col">
								<span class="group-label label" class:first={g === 0}>{group.label}</span>
								<ul class="flex flex-col gap-px">
									{#each group.pages as page (page.id)}
										{@const found = settingsNav.counts[page.id] ?? 0}
										{@const current = !searching && settingsNav.page === page.id}
										<li>
											<button
												type="button"
												class="page-item"
												class:current
												class:empty={searching && found === 0}
												aria-current={current ? 'page' : undefined}
												onclick={() => openSettingsPage(page.id)}
											>
												<span class="truncate">{page.label}</span>
												{#if searching && found > 0}
													<span class="tnum ml-auto text-xs text-subtle" aria-label="{found} matching">{found}</span>
												{/if}
											</button>
										</li>
									{/each}
								</ul>
							</li>
						{/each}
					</ul>
				{/if}
			</li>
		{/each}
	</ul>
	{#if importing || exports.running.length > 0}
		<div class="mt-auto flex flex-col gap-0.5 border-t border-line p-2" transition:fade={{ duration: reduceMotion ? 0 : 160 }}>
			{#if importing}
				<SidebarProgress
					label="Importing"
					percent={importPercent}
					detail="{importing.done} of {importing.total}{importLeft ? ` · ${importLeft}` : ''}"
					tooltip="Importing: {importing.done} of {importing.total} done, {importPercent}%"
					{collapsed}
					onclick={openImport}
				/>
			{/if}
			{#each exports.running as item (item.path)}
				{@const percent = Math.floor(item.fraction * 100)}
				{@const left = timeLeft(exportSecsLeft(item, now), true)}
				<SidebarProgress
					label="Exporting"
					{percent}
					detail="{item.name}{left ? ` · ${left}` : ''}"
					tooltip="Exporting {item.name}, {percent}%"
					{collapsed}
					onclick={() => openInEditor(item.path)}
				/>
			{/each}
		</div>
	{/if}
</nav>

<style>
	.sidebar {
		width: var(--sidebar-w);
	}
	.nav-item {
		display: flex;
		width: 100%;
		height: 28px;
		align-items: center;
		gap: 8px;
		border-radius: 4px;
		padding: 0 8px;
		font-size: 13px;
		color: var(--text-muted);
		transition:
			background-color var(--dur-fast) var(--ease-out),
			color var(--dur-fast) var(--ease-out),
			transform var(--dur-fast) var(--ease-out);
	}
	.nav-item:hover:not([aria-disabled='true']) {
		background: var(--surface-raised);
		color: var(--text);
	}
	.nav-item:active:not([aria-disabled='true']) {
		transform: scale(0.98);
	}
	.nav-item.active {
		background: var(--surface-selected);
		color: var(--text);
	}
	.nav-item[aria-disabled='true'] {
		color: var(--text-subtle);
		opacity: 0.6;
	}
	.pages {
		overflow: hidden;
	}
	.group-label {
		padding: 10px 8px 3px;
		font-size: 11px;
		color: var(--text-subtle);
	}
	.group-label.first {
		padding-top: 2px;
	}
	.page-item {
		position: relative;
		display: flex;
		width: 100%;
		height: 26px;
		align-items: center;
		gap: 8px;
		border-radius: 4px;
		padding: 0 8px;
		font-size: 13px;
		color: var(--text-muted);
		transition:
			background-color var(--dur-fast) var(--ease-out),
			color var(--dur-fast) var(--ease-out);
	}
	/* A hairline down the left ties the pages to Settings. */
	.page-item::before {
		content: '';
		position: absolute;
		left: -9px;
		top: 0;
		bottom: 0;
		width: 1px;
		background: var(--line);
	}
	.page-item.current::before {
		background: var(--text);
	}
	.page-item:hover {
		background: var(--surface-raised);
		color: var(--text);
	}
	.page-item.current {
		background: var(--surface-selected);
		color: var(--text);
	}
	.page-item.empty {
		opacity: 0.45;
	}
	.page-item:focus-visible {
		outline: 2px solid var(--focus);
		outline-offset: -2px;
	}
	.sidebar.collapsed {
		width: var(--sidebar-w-compact);
	}
	.sidebar.collapsed .nav-item {
		justify-content: center;
		padding: 0;
	}
	.sidebar.collapsed .label {
		display: none;
	}
	@media (max-width: 879px) {
		.sidebar {
			width: var(--sidebar-w-compact);
		}
		.nav-item {
			justify-content: center;
			padding: 0;
		}
		.label {
			display: none;
		}
	}
</style>
