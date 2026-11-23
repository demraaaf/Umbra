<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { FracRect } from '$lib/api/types';

	/**
	 * The whole source picture (the children) with the part kept marked: drag it to move it,
	 * drag a corner to resize it. The arrow keys move it; with Shift they resize it.
	 */
	let {
		rect = $bindable(),
		aspect,
		label,
		onchange,
		children
	}: {
		rect: FracRect;
		/** The source's width over its height. */
		aspect: number;
		label: string;
		onchange?: (rect: FracRect) => void;
		children: Snippet;
	} = $props();

	type Corner = 'nw' | 'ne' | 'sw' | 'se';
	const CORNERS: Corner[] = ['nw', 'ne', 'sw', 'se'];
	const MIN = 0.1;

	let width = $state(0);
	const height = $derived(width / aspect);
	const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

	let drag: { kind: 'move' | Corner; x: number; y: number; start: FracRect } | null = null;

	function begin(e: PointerEvent, kind: 'move' | Corner) {
		if (e.button !== 0) return;
		e.preventDefault();
		e.stopPropagation();
		(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
		drag = { kind, x: e.clientX, y: e.clientY, start: { ...rect } };
	}

	function move(e: PointerEvent) {
		if (!drag || width <= 0) return;
		const dx = (e.clientX - drag.x) / width;
		const dy = (e.clientY - drag.y) / height;
		const s = drag.start;
		if (drag.kind === 'move') {
			rect = { ...s, x: clamp(s.x + dx, 0, 1 - s.width), y: clamp(s.y + dy, 0, 1 - s.height) };
			return;
		}
		const right = s.x + s.width;
		const bottom = s.y + s.height;
		let { x, y } = s;
		let w = s.width;
		let h = s.height;
		if (drag.kind.includes('w')) {
			x = clamp(s.x + dx, 0, right - MIN);
			w = right - x;
		} else {
			w = clamp(s.width + dx, MIN, 1 - s.x);
		}
		if (drag.kind.includes('n')) {
			y = clamp(s.y + dy, 0, bottom - MIN);
			h = bottom - y;
		} else {
			h = clamp(s.height + dy, MIN, 1 - s.y);
		}
		rect = { x, y, width: w, height: h };
	}

	function end(e: PointerEvent) {
		if (!drag) return;
		(e.currentTarget as HTMLElement).releasePointerCapture?.(e.pointerId);
		drag = null;
		onchange?.(rect);
	}

	function onkeydown(e: KeyboardEvent) {
		const arrows: Record<string, [number, number]> = {
			ArrowLeft: [-1, 0],
			ArrowRight: [1, 0],
			ArrowUp: [0, -1],
			ArrowDown: [0, 1]
		};
		const dir = arrows[e.key];
		if (!dir) return;
		e.preventDefault();
		const step = e.ctrlKey || e.metaKey ? 0.05 : 0.01;
		if (e.shiftKey) {
			rect = {
				...rect,
				width: clamp(rect.width + dir[0] * step, MIN, 1 - rect.x),
				height: clamp(rect.height + dir[1] * step, MIN, 1 - rect.y)
			};
		} else {
			rect = {
				...rect,
				x: clamp(rect.x + dir[0] * step, 0, 1 - rect.width),
				y: clamp(rect.y + dir[1] * step, 0, 1 - rect.height)
			};
		}
		onchange?.(rect);
	}

	const px = $derived({
		left: rect.x * width,
		top: rect.y * height,
		width: rect.width * width,
		height: rect.height * height
	});
</script>

<div class="relative w-full overflow-hidden rounded-sm bg-black" style:height="{height}px" bind:clientWidth={width}>
	<div class="absolute inset-0">{@render children()}</div>
	<span class="dim" style:left="0" style:right="0" style:top="0" style:height="{px.top}px"></span>
	<span class="dim" style:left="0" style:right="0" style:top="{px.top + px.height}px" style:bottom="0"></span>
	<span class="dim" style:left="0" style:top="{px.top}px" style:width="{px.left}px" style:height="{px.height}px"></span>
	<span class="dim" style:left="{px.left + px.width}px" style:right="0" style:top="{px.top}px" style:height="{px.height}px"></span>
	<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
	<div
		class="kept"
		style:left="{px.left}px"
		style:top="{px.top}px"
		style:width="{px.width}px"
		style:height="{px.height}px"
		role="group"
		tabindex="0"
		aria-label="{label}: drag to move, drag a corner to resize; arrow keys move it, Shift and arrow keys resize it"
		onpointerdown={(e) => begin(e, 'move')}
		onpointermove={move}
		onpointerup={end}
		onpointercancel={end}
		{onkeydown}
	>
		{#each CORNERS as corner (corner)}
			<span
				class="corner"
				style:left={corner.includes('w') ? '0%' : '100%'}
				style:top={corner.includes('n') ? '0%' : '100%'}
				style:cursor={corner === 'nw' || corner === 'se' ? 'nwse-resize' : 'nesw-resize'}
				onpointerdown={(e) => begin(e, corner)}
				onpointermove={move}
				onpointerup={end}
				onpointercancel={end}
				aria-hidden="true"
			></span>
		{/each}
	</div>
</div>

<style>
	.dim {
		position: absolute;
		background: rgb(0 0 0 / 0.6);
		pointer-events: none;
	}
	.kept {
		position: absolute;
		cursor: move;
		touch-action: none;
		outline: 1px solid var(--focus);
	}
	.kept:focus-visible {
		outline-width: 2px;
	}
	.corner {
		position: absolute;
		width: 10px;
		height: 10px;
		margin: -5px 0 0 -5px;
		border: 1px solid #000;
		background: var(--focus);
	}
</style>
