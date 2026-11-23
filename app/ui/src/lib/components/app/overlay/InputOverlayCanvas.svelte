<script lang="ts">
	import type { InputOverlay } from '$lib/api/types';
	import { type Composition, type InputTimeline, drawOverlay } from '$lib/input/overlay';

	/**
	 * The keyboard and mouse at `time` (seconds of the clip), filling its box. The box has the
	 * composition's shape (`OverlayFrame` gives it); drawn sharp on any screen.
	 */
	let {
		timeline,
		composition,
		prefs,
		time
	}: { timeline: InputTimeline; composition: Composition; prefs: InputOverlay; time: number } = $props();

	let canvas = $state<HTMLCanvasElement | null>(null);
	let width = $state(0);
	let height = $state(0);

	$effect(() => {
		const c = canvas;
		if (!c || width <= 0 || height <= 0) return;
		const scale = window.devicePixelRatio || 1;
		const w = Math.round(width * scale);
		const h = Math.round(height * scale);
		if (c.width !== w) c.width = w;
		if (c.height !== h) c.height = h;
		const ctx = c.getContext('2d');
		if (!ctx) return;
		ctx.setTransform(scale, 0, 0, scale, 0, 0);
		drawOverlay(ctx, composition, timeline.visual(time, composition), width, prefs);
	});
</script>

<div class="size-full" bind:clientWidth={width} bind:clientHeight={height}>
	<canvas bind:this={canvas} class="block size-full" aria-hidden="true"></canvas>
</div>
