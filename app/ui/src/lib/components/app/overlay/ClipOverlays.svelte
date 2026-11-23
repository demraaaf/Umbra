<script lang="ts">
	import { api } from '$lib/api';
	import type { CameraVideo, Clip } from '$lib/api/types';
	import type { Playback } from '$lib/editor/playback.svelte';
	import { DEFAULT_CAMERA_PLACEMENT, parseLogo, parsePlacement } from '$lib/editor/overlay';
	import { DEFAULT_INPUT_OVERLAY, InputTimeline, compose, parseInputOverlay } from '$lib/input/overlay';
	import { app } from '$lib/stores/app.svelte';
	import CameraStandIn from './CameraStandIn.svelte';
	import InputOverlayCanvas from './InputOverlayCanvas.svelte';
	import OverlayFrame from './OverlayFrame.svelte';
	import SyncedCamera from './SyncedCamera.svelte';

	/**
	 * A clip's camera and logo where it was saved with them, over a video shown whole
	 * (`object-contain`) in a `width`×`height` area. Only to look at; the editor moves them.
	 */
	let { clip, playback, width, height }: { clip: Clip; playback: Playback; width: number; height: number } = $props();

	const box = $derived.by(() => {
		const aspect = clip.width / clip.height || 16 / 9;
		const w = Math.min(width, height * aspect);
		const h = w / aspect;
		return { left: (width - w) / 2, top: (height - h) / 2, width: w, height: h };
	});

	const cameraPlacement = $derived(clip.camera ? (parsePlacement(clip.details.camera) ?? DEFAULT_CAMERA_PLACEMENT) : null);
	const logo = $derived(parseLogo(clip.details.logo));
	const input = $derived(clip.input ? parseInputOverlay(clip.details.input, app.settings?.input.overlay ?? DEFAULT_INPUT_OVERLAY) : null);
	const inputComposition = $derived(input ? compose(input) : null);

	let inputTimeline = $state<InputTimeline | null>(null);
	$effect(() => {
		const path = clip.path;
		inputTimeline = null;
		if (!clip.input) return;
		let alive = true;
		void api
			.getInputTrack(path)
			.then((track) => alive && (inputTimeline = new InputTimeline(track)))
			.catch(() => {});
		return () => (alive = false);
	});

	let cameraVideo = $state<CameraVideo | null>(null);
	$effect(() => {
		const path = clip.path;
		cameraVideo = null;
		if (!clip.camera || playback.simulated) return;
		let alive = true;
		void api
			.getCameraVideo(path)
			.then((v) => alive && (cameraVideo = v))
			.catch(() => {});
		return () => (alive = false);
	});

	let logoUrl = $state<string | null>(null);
	let logoAspect = $state(3);
	$effect(() => {
		const path = logo?.path;
		logoUrl = null;
		if (path) void api.fileUrl(path).then((url) => (logoUrl = url));
	});

	const cameraShown = $derived.by(() => {
		const t = playback.time;
		if (!cameraVideo) return playback.simulated;
		return cameraVideo.stretches.some(([a, b]) => t >= a && t < b);
	});
</script>

{#if width > 0 && height > 0 && (cameraPlacement || logo || inputComposition)}
	<div
		class="pointer-events-none absolute"
		style:left="{box.left}px"
		style:top="{box.top}px"
		style:width="{box.width}px"
		style:height="{box.height}px"
	>
		{#if logo && logoUrl}
			<OverlayFrame placement={logo.placement} sourceAspect={logoAspect} frameWidth={box.width} frameHeight={box.height} label="Logo">
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
			</OverlayFrame>
		{/if}
		{#if input && inputComposition && inputTimeline}
			<OverlayFrame
				placement={input.placement}
				sourceAspect={inputComposition.width / inputComposition.height}
				frameWidth={box.width}
				frameHeight={box.height}
				label="Keyboard and mouse"
			>
				<InputOverlayCanvas timeline={inputTimeline} composition={inputComposition} prefs={input} time={playback.time} />
			</OverlayFrame>
		{/if}
		{#if cameraPlacement && clip.camera}
			<OverlayFrame
				placement={cameraPlacement}
				sourceAspect={clip.camera.width / clip.camera.height}
				frameWidth={box.width}
				frameHeight={box.height}
				opacity={cameraShown ? 1 : 0}
				label="Camera"
			>
				{#if cameraVideo}
					<SyncedCamera url={cameraVideo.url} offset={cameraVideo.stretches[0]?.[0] ?? 0} {playback} />
				{:else}
					<CameraStandIn />
				{/if}
			</OverlayFrame>
		{/if}
	</div>
{/if}
