<script lang="ts">
	import { api } from '$lib/api';
	import type { EditSession } from '$lib/editor/session.svelte';
	import { compose } from '$lib/input/overlay';
	import CameraStandIn from '../overlay/CameraStandIn.svelte';
	import InputOverlayCanvas from '../overlay/InputOverlayCanvas.svelte';
	import OverlayFrame from '../overlay/OverlayFrame.svelte';
	import SyncedCamera from '../overlay/SyncedCamera.svelte';

	/**
	 * The clip's camera and logo over the picture, as the export will draw them. `frameWidth`
	 * and `frameHeight` are the exported picture's box (the crop, when there is one). While
	 * `editable`, they can be moved and resized; where the camera is hidden it shows faintly.
	 */
	let {
		session,
		frameWidth,
		frameHeight,
		editable = false
	}: { session: EditSession; frameWidth: number; frameHeight: number; editable?: boolean } = $props();

	const playback = $derived(session.playback);
	const cameraShown = $derived(session.cameraShownAt(playback.time));
	const offset = $derived(session.cameraVideo?.stretches[0]?.[0] ?? 0);
	const inputComposition = $derived(session.input ? compose(session.input) : null);

	let logoUrl = $state<string | null>(null);
	let logoAspect = $state(3);
	$effect(() => {
		const path = session.logo?.path;
		logoUrl = null;
		if (path) void api.fileUrl(path).then((url) => (logoUrl = url));
	});
</script>

{#if frameWidth > 0 && frameHeight > 0}
	{#if session.logo && !session.logoOff}
		<OverlayFrame
			bind:placement={() => session.logo!.placement, (p) => session.setLogoPlacement(p)}
			sourceAspect={logoAspect}
			{frameWidth}
			{frameHeight}
			{editable}
			label="Logo"
			selected={session.overlaySelected === 'logo'}
			onselect={() => (session.overlaySelected = 'logo')}
		>
			{#if logoUrl}
				<img
					src={logoUrl}
					alt=""
					class="block size-full"
					draggable="false"
					onload={(e) => {
						const img = e.currentTarget as HTMLImageElement;
						if (img.naturalWidth > 0) logoAspect = img.naturalWidth / img.naturalHeight;
					}}
				/>
			{/if}
		</OverlayFrame>
	{/if}

	{#if session.input && !session.inputOff && inputComposition}
		<OverlayFrame
			bind:placement={() => session.input!.placement, (p) => session.setInputPlacement(p)}
			sourceAspect={inputComposition.width / inputComposition.height}
			{frameWidth}
			{frameHeight}
			{editable}
			label="Keyboard and mouse"
			selected={session.overlaySelected === 'input'}
			onselect={() => (session.overlaySelected = 'input')}
		>
			{#if session.inputTimeline}
				<InputOverlayCanvas timeline={session.inputTimeline} composition={inputComposition} prefs={session.input} time={playback.time} />
			{/if}
		</OverlayFrame>
	{/if}

	{#if session.cameraPlacement && !session.cameraOff}
		<OverlayFrame
			bind:placement={() => session.cameraPlacement!, (p) => session.setCameraPlacement(p)}
			sourceAspect={session.cameraAspect}
			{frameWidth}
			{frameHeight}
			{editable}
			opacity={cameraShown ? 1 : editable ? 0.25 : 0}
			label={cameraShown ? 'Camera' : 'Camera (hidden here)'}
			selected={session.overlaySelected === 'camera'}
			onselect={() => (session.overlaySelected = 'camera')}
		>
			{#if session.cameraVideo}
				<SyncedCamera url={session.cameraVideo.url} {offset} {playback} />
			{:else}
				<CameraStandIn />
			{/if}
		</OverlayFrame>
	{/if}
{/if}
