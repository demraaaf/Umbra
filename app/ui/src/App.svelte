<script lang="ts">
	import { onMount, untrack } from 'svelte';
	import { Toaster } from '$lib/components/ui/sonner';
	import * as Tooltip from '$lib/components/ui/tooltip';
	import TitleBar from '$lib/components/app/TitleBar.svelte';
	import Sidebar from '$lib/components/app/Sidebar.svelte';
	import ImportDialog from '$lib/components/app/clips/ImportDialog.svelte';
	import DropOverlay from '$lib/components/app/DropOverlay.svelte';
	import { initFileDrop } from '$lib/stores/importer.svelte';
	import RecordingPanel from '$lib/components/app/panel/RecordingPanel.svelte';
	import ClipProperties from '$lib/components/app/ClipProperties.svelte';
	import AlbumScreen from '$lib/screens/AlbumScreen.svelte';
	import AlbumsScreen from '$lib/screens/AlbumsScreen.svelte';
	import ClipsScreen from '$lib/screens/ClipsScreen.svelte';
	import EditorScreen from '$lib/screens/EditorScreen.svelte';
	import PlayerScreen from '$lib/screens/PlayerScreen.svelte';
	import SettingsScreen from '$lib/screens/SettingsScreen.svelte';
	import ViewerScreen from '$lib/screens/ViewerScreen.svelte';
	import OnboardingScreen from '$lib/screens/OnboardingScreen.svelte';
	import { app, boot } from '$lib/stores/app.svelte';
	import { followScreen, layout, toggleLeft, toggleRight } from '$lib/stores/layout.svelte';

	// The editor gets the room of the recording panel; it comes back when the editor is left.
	// Only a change of screen runs this; opening the panel again inside the editor sticks.
	$effect(() => {
		const screen = app.screen;
		untrack(() => followScreen(screen));
	});

	onMount(() => {
		let dispose: (() => void) | undefined;
		let cancelled = false;
		// Videos dragged in from Explorer open the import with them.
		void initFileDrop();
		void boot().then((d) => {
			if (cancelled) d();
			else dispose = d;
		});

		// Ctrl+B toggles the sidebar, Ctrl+Alt+B the recording panel.
		const onKey = (e: KeyboardEvent) => {
			if (e.defaultPrevented || !(e.ctrlKey || e.metaKey) || e.shiftKey || e.code !== 'KeyB') return;
			e.preventDefault();
			if (e.altKey) toggleRight();
			else toggleLeft();
		};
		window.addEventListener('keydown', onKey);

		return () => {
			cancelled = true;
			dispose?.();
			window.removeEventListener('keydown', onKey);
		};
	});
</script>

<Tooltip.Provider delayDuration={400} skipDelayDuration={300}>
	<div class="flex h-full flex-col overflow-clip bg-background">
		<TitleBar />
		{#if app.booted && app.firstRun && !app.fatal}
			<OnboardingScreen />
		{:else}
			<div class="flex min-h-0 flex-1">
				<Sidebar />
				<ImportDialog />
				<DropOverlay />
				<main class="relative flex min-w-0 flex-1 flex-col overflow-clip bg-canvas" id="main" tabindex="-1">
					{#if app.fatal}
						<div class="flex flex-1 items-center justify-center p-8">
							<div class="max-w-md text-center">
								<h1 class="text-base font-semibold">Umbra could not start</h1>
								<p class="selectable mt-2 text-sm text-muted-foreground">{app.fatal}</p>
							</div>
						</div>
					{:else if !app.booted}
						<div class="flex flex-1 items-center justify-center text-sm text-subtle" role="status">Loading</div>
					{:else if app.screen === 'clips'}
						<ClipsScreen />
					{:else if app.screen === 'player'}
						<PlayerScreen />
					{:else if app.screen === 'editor'}
						<EditorScreen />
					{:else if app.screen === 'viewer'}
						<ViewerScreen />
					{:else if app.screen === 'albums'}
						<AlbumsScreen />
					{:else if app.screen === 'album'}
						<AlbumScreen />
					{:else}
						<SettingsScreen />
					{/if}
				</main>
				{#if !layout.rightCollapsed && app.booted && !app.fatal}
					<RecordingPanel />
				{/if}
			</div>
		{/if}
	</div>
	<ClipProperties />
	<Toaster />
</Tooltip.Provider>
