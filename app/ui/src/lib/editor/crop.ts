// Crop rectangle geometry, in source pixels. Pure functions so the maths stays testable.
// Video encoders (4:2:0) need even sizes and offsets, so every result is snapped to even values.

import type { CropRect } from '$lib/api/types';

export const MIN_CROP = 16;

export type Handle = 'nw' | 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w';

export const HANDLES: Handle[] = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'];

export const ASPECTS = [
	{ key: 'free', label: 'Free', ratio: null },
	{ key: '16:9', label: '16:9', ratio: 16 / 9 },
	{ key: '4:3', label: '4:3', ratio: 4 / 3 },
	{ key: '1:1', label: '1:1', ratio: 1 },
	{ key: '9:16', label: '9:16', ratio: 9 / 16 }
] as const;

export type AspectKey = (typeof ASPECTS)[number]['key'];

export function aspectRatio(key: AspectKey): number | null {
	return ASPECTS.find((a) => a.key === key)?.ratio ?? null;
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
export const even = (v: number) => Math.round(v / 2) * 2;

export function fullFrame(width: number, height: number): CropRect {
	return { x: 0, y: 0, width, height };
}

export function isFullFrame(rect: CropRect, width: number, height: number): boolean {
	return rect.x === 0 && rect.y === 0 && rect.width >= width && rect.height >= height;
}

/** Snaps to even values and forces the rectangle inside the frame. */
export function snapRect(rect: CropRect, frameW: number, frameH: number): CropRect {
	const maxW = frameW - (frameW % 2);
	const maxH = frameH - (frameH % 2);
	const width = clamp(even(rect.width), Math.min(MIN_CROP, maxW), maxW);
	const height = clamp(even(rect.height), Math.min(MIN_CROP, maxH), maxH);
	const x = clamp(even(rect.x), 0, maxW - width);
	const y = clamp(even(rect.y), 0, maxH - height);
	return { x, y, width, height };
}

export function moveRect(rect: CropRect, dx: number, dy: number, frameW: number, frameH: number): CropRect {
	return snapRect(
		{
			...rect,
			x: clamp(rect.x + dx, 0, frameW - rect.width),
			y: clamp(rect.y + dy, 0, frameH - rect.height)
		},
		frameW,
		frameH
	);
}

/**
 * Drags one handle by (dx, dy). With an aspect ratio only corner handles apply: the opposite
 * corner stays put and the rectangle keeps its ratio.
 */
export function resizeRect(
	start: CropRect,
	handle: Handle,
	dx: number,
	dy: number,
	frameW: number,
	frameH: number,
	ratio: number | null
): CropRect {
	const west = handle.includes('w');
	const east = handle.includes('e');
	const north = handle.includes('n');
	const south = handle.includes('s');
	let left = start.x;
	let top = start.y;
	let right = start.x + start.width;
	let bottom = start.y + start.height;

	if (west) left = clamp(start.x + dx, 0, right - MIN_CROP);
	if (east) right = clamp(right + dx, left + MIN_CROP, frameW);
	if (north) top = clamp(start.y + dy, 0, bottom - MIN_CROP);
	if (south) bottom = clamp(bottom + dy, top + MIN_CROP, frameH);

	if (ratio && handle.length === 2) {
		const anchorX = west ? start.x + start.width : start.x;
		const anchorY = north ? start.y + start.height : start.y;
		let width = right - left;
		const height = bottom - top;
		width = Math.max(width, height * ratio);
		const maxWidth = Math.min(west ? anchorX : frameW - anchorX, (north ? anchorY : frameH - anchorY) * ratio);
		width = clamp(width, Math.max(MIN_CROP, MIN_CROP * ratio), Math.max(MIN_CROP, maxWidth));
		const h = width / ratio;
		if (west) left = anchorX - width;
		else right = anchorX + width;
		if (north) top = anchorY - h;
		else bottom = anchorY + h;
	}

	return snapRect({ x: left, y: top, width: right - left, height: bottom - top }, frameW, frameH);
}

/** Largest rectangle with the given ratio that fits inside `rect`, centred in it. */
export function fitAspect(rect: CropRect, ratio: number, frameW: number, frameH: number): CropRect {
	let width = rect.width;
	let height = width / ratio;
	if (height > rect.height) {
		height = rect.height;
		width = height * ratio;
	}
	return snapRect(
		{
			x: rect.x + (rect.width - width) / 2,
			y: rect.y + (rect.height - height) / 2,
			width,
			height
		},
		frameW,
		frameH
	);
}

/** Sets one numeric field, keeping the rectangle inside the frame (and on ratio, if locked). */
export function setField(
	rect: CropRect,
	field: keyof CropRect,
	value: number,
	frameW: number,
	frameH: number,
	ratio: number | null
): CropRect {
	const next = { ...rect };
	if (field === 'x') next.x = clamp(value, 0, frameW - rect.width);
	else if (field === 'y') next.y = clamp(value, 0, frameH - rect.height);
	else if (field === 'width') {
		next.width = clamp(value, MIN_CROP, frameW - rect.x);
		if (ratio) next.height = next.width / ratio;
	} else {
		next.height = clamp(value, MIN_CROP, frameH - rect.y);
		if (ratio) next.width = next.height * ratio;
	}
	if (ratio) {
		// The ratio can push a side past the frame edge; pull it back on ratio.
		const maxWidth = Math.min(frameW - next.x, (frameH - next.y) * ratio);
		if (next.width > maxWidth) {
			next.width = maxWidth;
			next.height = maxWidth / ratio;
		}
	}
	return snapRect(next, frameW, frameH);
}
