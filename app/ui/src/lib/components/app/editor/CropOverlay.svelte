<script lang="ts">
	import type { CropRect } from '$lib/api/types';
	import { HANDLES, aspectRatio, type Handle } from '$lib/editor/crop';
	import type { EditSession } from '$lib/editor/session.svelte';
	import { formatResolution } from '$lib/format/format';

	let { session, boxWidth }: { session: EditSession; boxWidth: number } = $props();

	const scale = $derived(boxWidth / session.clip.width);
	const crop = $derived(session.crop);
	const ratio = $derived(aspectRatio(session.aspect));
	// With a locked ratio only the corners resize; edges would break the ratio.
	const handles = $derived(ratio ? HANDLES.filter((h) => h.length === 2) : HANDLES);

	const CURSORS: Record<Handle, string> = {
		nw: 'nwse-resize',
		se: 'nwse-resize',
		ne: 'nesw-resize',
		sw: 'nesw-resize',
		n: 'ns-resize',
		s: 'ns-resize',
		e: 'ew-resize',
		w: 'ew-resize'
	};

	function position(handle: Handle): string {
		const x = handle.includes('w') ? '0%' : handle.includes('e') ? '100%' : '50%';
		const y = handle.includes('n') ? '0%' : handle.includes('s') ? '100%' : '50%';
		return `left:${x};top:${y}`;
	}

	let drag: { handle: Handle | 'move'; x: number; y: number; start: CropRect } | null = null;

	function begin(e: PointerEvent, handle: Handle | 'move') {
		if (e.button !== 0 || session.busy) return;
		e.preventDefault();
		e.stopPropagation();
		(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
		drag = { handle, x: e.clientX, y: e.clientY, start: { ...session.crop } };
	}

	function move(e: PointerEvent) {
		if (!drag) return;
		session.dragCrop(drag.start, drag.handle, (e.clientX - drag.x) / scale, (e.clientY - drag.y) / scale);
	}

	function end(e: PointerEvent) {
		if (!drag) return;
		(e.currentTarget as HTMLElement).releasePointerCapture?.(e.pointerId);
		drag = null;
	}

	function onkeydown(e: KeyboardEvent) {
		const arrows: Record<string, [number, number]> = {
			ArrowLeft: [-1, 0],
			ArrowRight: [1, 0],
			ArrowUp: [0, -1],
			ArrowDown: [0, 1]
		};
		const dir = arrows[e.key];
		if (!dir || session.busy) return;
		e.preventDefault();
		const amount = (e.ctrlKey || e.metaKey ? 20 : 2);
		if (e.shiftKey) {
			// Shift resizes from the bottom-right corner.
			session.setCropField('width', session.crop.width + dir[0] * amount);
			if (!ratio) session.setCropField('height', session.crop.height + dir[1] * amount);
		} else {
			session.moveCrop(dir[0] * amount, dir[1] * amount);
		}
	}
</script>

<div class="pointer-events-none absolute inset-0 overflow-hidden" data-crop-overlay>
	<!-- Everything outside the crop is dimmed with four plain rectangles. -->
	<span class="dim" style:left="0" style:right="0" style:top="0" style:height="{crop.y * scale}px"></span>
	<span class="dim" style:left="0" style:right="0" style:top="{(crop.y + crop.height) * scale}px" style:bottom="0"></span>
	<span class="dim" style:left="0" style:top="{crop.y * scale}px" style:width="{crop.x * scale}px" style:height="{crop.height * scale}px"></span>
	<span class="dim" style:left="{(crop.x + crop.width) * scale}px" style:right="0" style:top="{crop.y * scale}px" style:height="{crop.height * scale}px"></span>

	<!-- The crop box is a custom widget: draggable, resizable, and operable with arrow keys. -->
	<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
	<div
		class="crop pointer-events-auto absolute touch-none"
		style:left="{crop.x * scale}px"
		style:top="{crop.y * scale}px"
		style:width="{crop.width * scale}px"
		style:height="{crop.height * scale}px"
		role="application"
		tabindex="0"
		aria-label="Crop area, {crop.width} by {crop.height}, at {crop.x}, {crop.y}. Arrow keys move it, Shift with arrow keys resizes it."
		onpointerdown={(e) => begin(e, 'move')}
		onpointermove={move}
		onpointerup={end}
		onpointercancel={end}
		{onkeydown}
	>
		<span class="guide guide-v" style:left="33.333%"></span>
		<span class="guide guide-v" style:left="66.666%"></span>
		<span class="guide guide-h" style:top="33.333%"></span>
		<span class="guide guide-h" style:top="66.666%"></span>

		<span class="readout tnum">{formatResolution(crop.width, crop.height)}</span>

		{#each handles as handle (handle)}
			<span
				class="handle"
				style={position(handle)}
				style:cursor={CURSORS[handle]}
				data-handle={handle}
				onpointerdown={(e) => begin(e, handle)}
				onpointermove={move}
				onpointerup={end}
				onpointercancel={end}
				role="presentation"
			></span>
		{/each}
	</div>
</div>

<style>
	.crop {
		box-sizing: border-box;
		border: 1px solid #f5f5f5;
		cursor: move;
		outline: none;
	}
	.crop:focus-visible {
		outline: 2px solid var(--focus);
		outline-offset: 2px;
	}
	.dim {
		position: absolute;
		background: rgb(0 0 0 / 0.62);
		pointer-events: none;
	}
	.guide {
		position: absolute;
		background: rgb(255 255 255 / 0.22);
		pointer-events: none;
	}
	.guide-v {
		top: 0;
		bottom: 0;
		width: 1px;
	}
	.guide-h {
		left: 0;
		right: 0;
		height: 1px;
	}
	.readout {
		position: absolute;
		left: 4px;
		top: 4px;
		padding: 0 4px;
		border-radius: 2px;
		background: rgb(0 0 0 / 0.75);
		color: var(--text);
		font-size: 12px;
		line-height: 16px;
		pointer-events: none;
	}
	.handle {
		position: absolute;
		width: 12px;
		height: 12px;
		margin: -6px 0 0 -6px;
		touch-action: none;
	}
	.handle::after {
		content: '';
		position: absolute;
		inset: 2px;
		background: #f5f5f5;
		border: 1px solid #000;
		border-radius: 1px;
	}
</style>
