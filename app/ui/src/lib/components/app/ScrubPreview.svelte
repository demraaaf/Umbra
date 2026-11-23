<script lang="ts">
	import { formatTimecode } from '$lib/format/format';

	/**
	 * The frame under the pointer, above a position bar. A hidden copy of the video seeks to
	 * the hovered time and the frame is drawn into a small canvas. Seeks never pile up: while
	 * one is running only the latest wanted time is kept.
	 */
	let {
		url,
		time,
		x,
		barWidth
	}: {
		url: string;
		/** Seconds under the pointer, or null when it is not over the bar. */
		time: number | null;
		/** Pointer position on the bar, in pixels. */
		x: number;
		barWidth: number;
	} = $props();

	const WIDTH = 176;

	let source = $state<HTMLVideoElement | null>(null);
	let canvas = $state<HTMLCanvasElement | null>(null);
	let height = $state(99);
	let drawn = $state(false);
	let shownTime = $state(0);
	let seeking = false;
	let wanted: number | null = null;

	function seek(t: number) {
		if (!source || source.readyState < 1) {
			wanted = t;
			return;
		}
		if (seeking) {
			wanted = t;
			return;
		}
		seeking = true;
		wanted = null;
		source.currentTime = t;
	}

	function draw() {
		seeking = false;
		if (source && canvas) {
			const ratio = source.videoHeight / Math.max(1, source.videoWidth);
			height = Math.round(WIDTH * (ratio || 9 / 16));
			canvas.width = WIDTH * 2;
			canvas.height = height * 2;
			canvas.getContext('2d')?.drawImage(source, 0, 0, canvas.width, canvas.height);
			shownTime = source.currentTime;
			drawn = true;
		}
		if (wanted !== null) seek(wanted);
	}

	$effect(() => {
		if (time === null) {
			drawn = false;
			wanted = null;
			return;
		}
		seek(time);
	});

	const left = $derived(Math.min(Math.max(x, WIDTH / 2), Math.max(WIDTH / 2, barWidth - WIDTH / 2)));
</script>

<!-- Only for its frames: never shown, never heard. -->
<video
	bind:this={source}
	src={url}
	class="hidden"
	muted
	preload="metadata"
	aria-hidden="true"
	onloadedmetadata={() => wanted !== null && seek(wanted)}
	onseeked={draw}
></video>

{#if time !== null}
	<span class="preview" class:ready={drawn} style:left="{left}px" style:width="{WIDTH}px" aria-hidden="true">
		<canvas bind:this={canvas} class="block w-full rounded-t-[3px] bg-black" style:height="{height}px"></canvas>
		<span class="tnum block px-1.5 py-0.5 text-center text-xs">{formatTimecode(drawn ? shownTime : time)}</span>
	</span>
{/if}

<style>
	.preview {
		position: absolute;
		bottom: calc(100% + 6px);
		z-index: 10;
		transform: translateX(-50%);
		overflow: hidden;
		border-radius: 4px;
		border: 1px solid var(--line-strong);
		background: var(--surface-panel);
		color: var(--text);
		box-shadow: 0 4px 14px rgb(0 0 0 / 0.5);
		pointer-events: none;
	}
	/* Until the first frame arrives only the time shows. */
	.preview:not(.ready) canvas {
		visibility: hidden;
	}
</style>
