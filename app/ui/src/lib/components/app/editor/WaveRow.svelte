<script lang="ts">
	import type { Range } from '$lib/editor/session.svelte';

	/**
	 * One audio track's outline on the timeline's scale. Parts outside the trim or cut out
	 * are drawn faint, the same way the clip strip above dims them.
	 */
	let {
		levels,
		sliceSecs,
		start,
		span,
		width,
		inSecs,
		outSecs,
		cuts,
		title,
		muted = false
	}: {
		levels: number[];
		sliceSecs: number;
		start: number;
		span: number;
		width: number;
		inSecs: number;
		outSecs: number;
		cuts: Range[];
		title: string;
		muted?: boolean;
	} = $props();

	const HEIGHT = 28;
	let canvas = $state<HTMLCanvasElement | null>(null);

	$effect(() => {
		const el = canvas;
		if (!el || width <= 0 || span <= 0) return;
		const dpr = window.devicePixelRatio || 1;
		el.width = Math.round(width * dpr);
		el.height = Math.round(HEIGHT * dpr);
		const ctx = el.getContext('2d');
		if (!ctx) return;
		ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
		ctx.clearRect(0, 0, width, HEIGHT);

		const style = getComputedStyle(el);
		const strong = style.getPropertyValue('--text-muted').trim() || '#a1a1a1';
		const faint = style.getPropertyValue('--line-control').trim() || '#3a3a3a';
		const mid = HEIGHT / 2;
		const secsPerPx = span / width;
		const kept = new Path2D();
		const dropped = new Path2D();
		for (let x = 0; x < width; x++) {
			const t0 = start + x * secsPerPx;
			const t1 = t0 + secsPerPx;
			const i0 = Math.max(0, Math.floor(t0 / sliceSecs));
			const i1 = Math.min(levels.length, Math.max(i0 + 1, Math.ceil(t1 / sliceSecs)));
			let peak = 0;
			for (let i = i0; i < i1; i++) if (levels[i] > peak) peak = levels[i];
			if (peak === 0) continue;
			const h = Math.max(0.5, (peak / 255) * (mid - 1));
			const t = (t0 + t1) / 2;
			const out = muted || t < inSecs || t > outSecs || cuts.some((c) => t >= c.start && t < c.end);
			(out ? dropped : kept).rect(x, mid - h, 1, h * 2);
		}
		ctx.fillStyle = faint;
		ctx.fill(dropped);
		ctx.fillStyle = strong;
		ctx.fill(kept);
	});
</script>

<div class="relative h-7 border-t border-line">
	<canvas bind:this={canvas} class="absolute inset-0 block" style:width="{width}px" style:height="{HEIGHT}px" aria-hidden="true"></canvas>
	<span class="label pointer-events-none absolute top-0.5 left-1 truncate text-[11px] leading-4 text-subtle">{title}</span>
</div>

<style>
	.label {
		max-width: 40%;
		padding: 0 4px;
		border-radius: 2px;
		background: rgb(10 10 10 / 0.7);
	}
</style>
