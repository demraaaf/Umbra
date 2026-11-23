<script lang="ts">
	import { onMount } from 'svelte';
	import PanelLeftIcon from '@lucide/svelte/icons/panel-left';
	import PanelRightIcon from '@lucide/svelte/icons/panel-right';
	import { getWindowControls, type WindowControls } from '$lib/api/window';
	import { app, currentAlbum, currentClip, playerClip, viewerClip } from '$lib/stores/app.svelte';
	import { SHORTCUT_LEFT, SHORTCUT_RIGHT, layout, toggleLeft, toggleRight } from '$lib/stores/layout.svelte';
	import { clipTitle } from '$lib/format/format';
	import IconButton from './IconButton.svelte';

	let controls: WindowControls | null = null;
	let maximized = $state(false);

	const title = $derived.by(() => {
		switch (app.screen) {
			case 'settings':
				return 'Settings';
			case 'albums':
				return 'Albums';
			case 'album':
				return currentAlbum()?.name ?? 'Album';
			case 'editor': {
				const clip = currentClip();
				return clip ? clipTitle(clip) : 'Editor';
			}
			case 'viewer': {
				const clip = viewerClip();
				return clip ? clipTitle(clip) : 'Screenshot';
			}
			case 'player': {
				const clip = playerClip();
				return clip ? clipTitle(clip) : 'Player';
			}
			default:
				return 'Clips';
		}
	});

	onMount(() => {
		let unlisten: (() => void) | undefined;
		let cancelled = false;
		void getWindowControls().then(async (c) => {
			controls = c;
			maximized = await c.isMaximized();
			const off = await c.onResized(async () => {
				maximized = await c.isMaximized();
			});
			if (cancelled) off();
			else unlisten = off;
		});
		return () => {
			cancelled = true;
			unlisten?.();
		};
	});

	async function toggleMaximize() {
		await controls?.toggleMaximize();
		maximized = (await controls?.isMaximized()) ?? maximized;
	}
</script>

<header
	class="flex h-(--titlebar-h) shrink-0 items-stretch border-b border-line bg-chrome text-foreground"
	data-tauri-drag-region
>
	<div
		class="brand flex shrink-0 items-center gap-2 px-3"
		class:collapsed={layout.leftCollapsed}
		data-tauri-drag-region
	>
		<svg class="pointer-events-none size-4 shrink-0" viewBox="0 0 16 16" aria-hidden="true">
			<circle cx="8" cy="8" r="6.5" fill="none" stroke="currentColor" stroke-width="1.5" />
			<path d="M8 1.5a6.5 6.5 0 0 0 0 13z" fill="currentColor" />
		</svg>
		<span class="brand-name pointer-events-none text-sm font-semibold tracking-tight" data-tauri-drag-region>Umbra</span>
	</div>

	<!-- The first-start screen has no sidebars and no screen to name. -->
	<div class="flex items-center pl-1" class:invisible={app.firstRun}>
		<IconButton
			label={layout.leftCollapsed ? 'Expand the sidebar' : 'Collapse the sidebar'}
			shortcut={SHORTCUT_LEFT}
			icon={PanelLeftIcon}
			size="icon-xs"
			pressed={!layout.leftCollapsed}
			onclick={toggleLeft}
		/>
	</div>

	<div class="pointer-events-none flex min-w-0 flex-1 items-center px-3 text-xs text-muted-foreground" data-tauri-drag-region>
		<span class="truncate" data-tauri-drag-region>{app.firstRun ? 'Setup' : title}</span>
	</div>

	<div class="flex items-center pr-2" class:invisible={app.firstRun}>
		<IconButton
			label={layout.rightCollapsed ? 'Show the recording panel' : 'Hide the recording panel'}
			shortcut={SHORTCUT_RIGHT}
			icon={PanelRightIcon}
			size="icon-xs"
			pressed={!layout.rightCollapsed}
			onclick={toggleRight}
		/>
	</div>

	<div class="flex shrink-0 items-stretch" role="group" aria-label="Window controls">
		<button type="button" class="win-btn" aria-label="Minimize" title="Minimize" onclick={() => controls?.minimize()}>
			<svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true"><path d="M0 5.5h10" stroke="currentColor" stroke-width="1" fill="none" /></svg>
		</button>
		<button
			type="button"
			class="win-btn"
			aria-label={maximized ? 'Restore' : 'Maximize'}
			title={maximized ? 'Restore' : 'Maximize'}
			onclick={toggleMaximize}
		>
			{#if maximized}
				<svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true">
					<path d="M2.5 2.5V.5h7v7h-2" stroke="currentColor" stroke-width="1" fill="none" />
					<rect x="0.5" y="2.5" width="7" height="7" stroke="currentColor" stroke-width="1" fill="none" />
				</svg>
			{:else}
				<svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true">
					<rect x="0.5" y="0.5" width="9" height="9" stroke="currentColor" stroke-width="1" fill="none" />
				</svg>
			{/if}
		</button>
		<button type="button" class="win-btn" aria-label="Close" title="Close" onclick={() => controls?.close()}>
			<svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true">
				<path d="M.5.5l9 9M9.5.5l-9 9" stroke="currentColor" stroke-width="1" fill="none" />
			</svg>
		</button>
	</div>
</header>

<style>
	.brand {
		width: var(--sidebar-w);
	}
	.brand.collapsed {
		width: var(--sidebar-w-compact);
		justify-content: center;
		padding: 0;
	}
	.brand.collapsed .brand-name {
		display: none;
	}
	@media (max-width: 879px) {
		.brand {
			width: var(--sidebar-w-compact);
			justify-content: center;
			padding: 0;
		}
		.brand-name {
			display: none;
		}
	}
	.win-btn {
		display: inline-flex;
		width: 46px;
		align-items: center;
		justify-content: center;
		color: var(--text-muted);
		background: transparent;
		transition:
			background-color var(--dur-fast) var(--ease-out),
			color var(--dur-fast) var(--ease-out);
	}
	.win-btn:hover {
		background: var(--surface-selected);
		color: var(--text);
	}
	.win-btn:active {
		background: #2b2b2b;
	}
	.win-btn:focus-visible {
		outline-offset: -2px;
	}
</style>
