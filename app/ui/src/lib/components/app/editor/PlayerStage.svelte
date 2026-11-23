<script lang="ts">
	import BufferingIndicator from '$lib/components/app/BufferingIndicator.svelte';
	import type { EditSession } from '$lib/editor/session.svelte';
	import { formatTimecode } from '$lib/format/format';
	import CropOverlay from './CropOverlay.svelte';
	import StageOverlays from './StageOverlays.svelte';

	/** `fade`: how much of the picture shows at the playhead, 0 black to 1 as it is. */
	let { session, fade = 1 }: { session: EditSession; fade?: number } = $props();

	const playback = $derived(session.playback);
	const clip = $derived(session.clip);

	let areaWidth = $state(0);
	let areaHeight = $state(0);
	let video = $state<HTMLVideoElement | null>(null);

	const PADDING = 16;

	/** The largest box of the clip's aspect ratio that fits the stage. */
	const box = $derived.by(() => {
		const aspect = clip.width / clip.height;
		const w = Math.max(0, areaWidth - PADDING * 2);
		const h = Math.max(0, areaHeight - PADDING * 2);
		const width = Math.floor(Math.min(w, h * aspect));
		return { width, height: Math.floor(width / aspect) };
	});

	$effect(() => {
		playback.attach(video);
		return () => playback.attach(null);
	});

	/** The exported picture within the box: the crop when one is set, the whole frame otherwise. */
	const frame = $derived.by(() => {
		if (!session.cropActive || box.width === 0) return { left: 0, top: 0, width: box.width, height: box.height };
		const scale = box.width / clip.width;
		const c = session.crop;
		return { left: c.x * scale, top: c.y * scale, width: c.width * scale, height: c.height * scale };
	});

</script>

<div
	class="relative flex min-h-0 min-w-0 flex-1 items-center justify-center bg-black"
	bind:clientWidth={areaWidth}
	bind:clientHeight={areaHeight}
>
	<div class="relative shrink-0 bg-black" style:width="{box.width}px" style:height="{box.height}px">
		{#if playback.simulated}
			<!-- Mock backend: there is no media file, so the frame is the thumbnail and time is a clock. -->
			<button
				type="button"
				class="absolute inset-0 block size-full cursor-default overflow-hidden bg-raised"
				aria-label={playback.playing ? 'Pause' : 'Play'}
				onclick={() => playback.toggle()}
			>
				{#if clip.thumbnailUrl}
					<img src={clip.thumbnailUrl} alt="" class="size-full object-cover" draggable="false" />
				{/if}
				<span class="tnum absolute top-2 left-2 rounded-xs bg-black/75 px-1 text-xs text-foreground">
					{formatTimecode(playback.time)}
				</span>
				<span class="absolute bottom-2 left-2 rounded-xs bg-black/75 px-1 text-xs text-muted-foreground">
					Simulated playback (mock data, no video file)
				</span>
			</button>
		{:else}
			{#key clip.videoUrl}
				<!-- svelte-ignore a11y_media_has_caption -->
				<video
					bind:this={video}
					src={clip.videoUrl}
					class="absolute inset-0 size-full bg-black object-contain"
					preload="auto"
					playsinline
					disablepictureinpicture
					disableremoteplayback
					aria-label="Clip preview"
					onloadedmetadata={() => {
						playback.onMetadata();
						session.onDuration(playback.duration);
					}}
					onerror={() => playback.onError()}
					onclick={() => playback.toggle()}
				></video>
			{/key}
		{/if}

		<!-- The camera and logo sit on the exported picture: inside the crop, when there is one. -->
		<div
			class="pointer-events-none absolute"
			style:left="{frame.left}px"
			style:top="{frame.top}px"
			style:width="{frame.width}px"
			style:height="{frame.height}px"
		>
			<StageOverlays {session} frameWidth={frame.width} frameHeight={frame.height} editable={!session.busy} />
		</div>

		{#if fade < 0.999}
			<!-- The fade the export will have here, previewed. -->
			<div class="pointer-events-none absolute inset-0 bg-black" style:opacity={1 - fade} aria-hidden="true"></div>
		{/if}

		{#if session.cropEnabled && box.width > 0}
			<CropOverlay {session} boxWidth={box.width} />
		{/if}

		{#if playback.error}
			<div class="absolute inset-0 flex items-center justify-center bg-black/85 p-6 text-center">
				<p class="max-w-xs text-sm text-muted-foreground" role="alert">{playback.error}</p>
			</div>
		{:else if !playback.ready}
			<div class="absolute inset-0 flex items-center justify-center text-sm text-subtle" role="status">Loading video</div>
		{:else}
			<BufferingIndicator active={playback.buffering} />
		{/if}
	</div>
</div>
