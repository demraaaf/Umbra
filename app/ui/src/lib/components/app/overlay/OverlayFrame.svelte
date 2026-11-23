<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { Placement } from '$lib/api/types';
	import { cornerRadius, overlayBox, shownCrop } from '$lib/editor/overlay';

	/**
	 * A picture over a video, where `placement` puts it: the camera or the logo. The children
	 * draw the whole source picture; this crops, mirrors, rounds and borders it. While
	 * `editable`, a drag moves it, a corner resizes it (keeping its shape), and the arrow keys
	 * move it (with Shift, they resize it). Edges and the middle pull it in; Alt drags freely.
	 */
	let {
		placement = $bindable(),
		sourceAspect,
		frameWidth,
		frameHeight,
		editable = false,
		label,
		selected = false,
		onselect,
		onchange,
		opacity = 1,
		children
	}: {
		placement: Placement;
		/** The source picture's width over its height. */
		sourceAspect: number;
		/** The video's box, in pixels. */
		frameWidth: number;
		frameHeight: number;
		editable?: boolean;
		label: string;
		selected?: boolean;
		onselect?: () => void;
		/** After a drag or a key: the placement as it now is. */
		onchange?: (placement: Placement) => void;
		/** Below 1 it shows faintly; at 0 it is hidden. Clicks go through unless it is editable. */
		opacity?: number;
		children: Snippet;
	} = $props();

	type Corner = 'nw' | 'ne' | 'sw' | 'se';
	const CORNERS: Corner[] = ['nw', 'ne', 'sw', 'se'];
	const MIN_WIDTH = 0.03;
	const SNAP_PX = 6;
	/** The gap to the video's edge that a drag snaps to, as at the default position. */
	const MARGIN = 0.02;

	const box = $derived(overlayBox(placement, sourceAspect, frameWidth, frameHeight));
	const crop = $derived(shownCrop(placement, sourceAspect));
	const radius = $derived(cornerRadius(placement, box.width, box.height));
	const borderPx = $derived(placement.border * box.width);
	/** The whole source, scaled so the crop fills the box. */
	const inner = $derived({
		width: box.width / crop.width,
		height: box.height / crop.height,
		left: (-crop.x * box.width) / crop.width,
		top: (-crop.y * box.height) / crop.height
	});

	const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
	/** Height as a fraction of the video for a width fraction. */
	const heightOf = (width: number) => (width * frameWidth) / (box.width / box.height || 1) / frameHeight;

	/** Pulls `v` (a fraction) onto the closest of `points` within a few pixels. */
	function snap(v: number, points: number[], px: number): number {
		const reach = SNAP_PX / px;
		let best = v;
		let bestDistance = reach;
		for (const p of points) {
			const d = Math.abs(v - p);
			if (d < bestDistance) {
				best = p;
				bestDistance = d;
			}
		}
		return best;
	}

	let drag: { kind: 'move' | Corner; x: number; y: number; start: Placement; startHeight: number } | null = null;

	function begin(e: PointerEvent, kind: 'move' | Corner) {
		if (!editable || e.button !== 0) return;
		e.preventDefault();
		e.stopPropagation();
		onselect?.();
		(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
		drag = { kind, x: e.clientX, y: e.clientY, start: { ...placement }, startHeight: heightOf(placement.width) };
	}

	function move(e: PointerEvent) {
		if (!drag || frameWidth <= 0 || frameHeight <= 0) return;
		const dx = (e.clientX - drag.x) / frameWidth;
		const dy = (e.clientY - drag.y) / frameHeight;
		const s = drag.start;
		const free = e.altKey;
		if (drag.kind === 'move') {
			const h = drag.startHeight;
			let x = clamp(s.x + dx, 0, 1 - s.width);
			let y = clamp(s.y + dy, 0, Math.max(0, 1 - h));
			if (!free) {
				x = snap(x, [0, MARGIN, 0.5 - s.width / 2, 1 - s.width - MARGIN, 1 - s.width], frameWidth);
				y = snap(y, [0, MARGIN * (frameWidth / frameHeight), 0.5 - h / 2, 1 - h - MARGIN * (frameWidth / frameHeight), 1 - h], frameHeight);
			}
			placement = { ...placement, x, y };
			return;
		}
		const west = drag.kind.includes('w');
		const north = drag.kind.includes('n');
		// The opposite corner stays where it is.
		const right = s.x + s.width;
		const bottom = s.y + drag.startHeight;
		const ratio = drag.startHeight / s.width;
		const maxWidth = Math.min(west ? right : 1 - s.x, (north ? bottom : 1 - s.y) / ratio);
		const width = clamp(s.width + (west ? -dx : dx), MIN_WIDTH, maxWidth);
		placement = {
			...placement,
			width,
			x: west ? right - width : s.x,
			y: north ? bottom - width * ratio : s.y
		};
	}

	function end(e: PointerEvent) {
		if (!drag) return;
		(e.currentTarget as HTMLElement).releasePointerCapture?.(e.pointerId);
		drag = null;
		onchange?.(placement);
	}

	function onkeydown(e: KeyboardEvent) {
		const arrows: Record<string, [number, number]> = {
			ArrowLeft: [-1, 0],
			ArrowRight: [1, 0],
			ArrowUp: [0, -1],
			ArrowDown: [0, 1]
		};
		const dir = arrows[e.key];
		if (!dir || !editable) return;
		e.preventDefault();
		const step = e.ctrlKey || e.metaKey ? 0.05 : 0.005;
		if (e.shiftKey) {
			const ratio = heightOf(placement.width) / placement.width;
			const grow = dir[0] !== 0 ? dir[0] : -dir[1];
			const width = clamp(placement.width + grow * step, MIN_WIDTH, Math.min(1 - placement.x, (1 - placement.y) / ratio));
			placement = { ...placement, width };
		} else {
			const h = heightOf(placement.width);
			placement = {
				...placement,
				x: clamp(placement.x + dir[0] * step, 0, 1 - placement.width),
				y: clamp(placement.y + dir[1] * step, 0, Math.max(0, 1 - h))
			};
		}
		onchange?.(placement);
	}

	function position(corner: Corner): string {
		return `left:${corner.includes('w') ? '0%' : '100%'};top:${corner.includes('n') ? '0%' : '100%'}`;
	}
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div
	class="frame absolute"
	class:editable
	class:selected={editable && selected}
	style:left="{box.left}px"
	style:top="{box.top}px"
	style:width="{box.width}px"
	style:height="{box.height}px"
	style:opacity={opacity < 1 ? opacity : undefined}
	style:pointer-events={editable && opacity > 0 ? 'auto' : 'none'}
	role={editable ? 'group' : undefined}
	aria-label={editable ? `${label}: drag to move, drag a corner to resize; arrow keys move it, Shift and arrow keys resize it` : label}
	tabindex={editable ? 0 : undefined}
	onpointerdown={(e) => begin(e, 'move')}
	onpointermove={move}
	onpointerup={end}
	onpointercancel={end}
	{onkeydown}
	onfocus={() => editable && onselect?.()}
>
	<div class="picture" style:border-radius="{radius}px" style:transform={placement.mirror ? 'scaleX(-1)' : undefined}>
		<div
			class="source"
			style:width="{inner.width}px"
			style:height="{inner.height}px"
			style:left="{inner.left}px"
			style:top="{inner.top}px"
		>
			{@render children()}
		</div>
	</div>
	{#if borderPx > 0.5}
		<div
			class="border-ring"
			style:border-radius="{radius}px"
			style:border="{borderPx}px solid {placement.borderColor}"
			aria-hidden="true"
		></div>
	{/if}
	{#if editable && selected}
		{#each CORNERS as corner (corner)}
			<span
				class="corner"
				style={position(corner)}
				style:cursor={corner === 'nw' || corner === 'se' ? 'nwse-resize' : 'nesw-resize'}
				onpointerdown={(e) => begin(e, corner)}
				onpointermove={move}
				onpointerup={end}
				onpointercancel={end}
				aria-hidden="true"
			></span>
		{/each}
	{/if}
</div>

<style>
	.frame {
		touch-action: none;
	}
	.frame.editable {
		cursor: move;
	}
	.frame:focus-visible {
		outline: 2px solid var(--focus);
		outline-offset: 2px;
	}
	.frame.selected {
		outline: 1px solid var(--focus);
		outline-offset: 1px;
	}
	.picture {
		position: absolute;
		inset: 0;
		overflow: hidden;
	}
	.source {
		position: absolute;
	}
	.border-ring {
		position: absolute;
		inset: 0;
		pointer-events: none;
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
