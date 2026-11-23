// The camera and the logo over a video: where they sit and what part of them shows. Shared by
// the settings preview, the player and the editor; export draws the same thing in Rust
// (`settings::Placement`), so the arithmetic here has to match it.

import type { FracRect, LogoSettings, Placement } from '$lib/api/types';

export const WHOLE: FracRect = { x: 0, y: 0, width: 1, height: 1 };

/** Bottom right, a fifth of the width wide (as `Placement::default` in Rust). */
export const DEFAULT_CAMERA_PLACEMENT: Placement = {
	x: 0.78,
	y: 0.75,
	width: 0.2,
	crop: { ...WHOLE },
	shape: 'rounded',
	border: 0,
	borderColor: '#ffffff',
	mirror: false
};

/** Top left, small (as `LogoPrefs::default` in Rust). */
export const DEFAULT_LOGO_PLACEMENT: Placement = {
	...DEFAULT_CAMERA_PLACEMENT,
	x: 0.02,
	y: 0.03,
	width: 0.1,
	shape: 'rectangle'
};

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/**
 * The part of the source shown: the crop, made square (about its middle) for a circle so it
 * is round and not an oval. `aspect` is the source's width over its height.
 */
export function shownCrop(p: Placement, aspect: number): FracRect {
	const c = p.crop;
	if (p.shape !== 'circle') return c;
	const w = c.width * aspect;
	const h = c.height;
	if (w > h) {
		const width = h / aspect;
		return { ...c, x: c.x + (c.width - width) / 2, width };
	}
	const height = w;
	return { ...c, y: c.y + (c.height - height) / 2, height };
}

/** Width over height of what shows: the source's aspect through the crop. */
export function shownAspect(p: Placement, sourceAspect: number): number {
	const c = shownCrop(p, sourceAspect);
	return (c.width * sourceAspect) / c.height;
}

/** The overlay's box in pixels of a `frameW`×`frameH` video, kept inside it. */
export function overlayBox(p: Placement, sourceAspect: number, frameW: number, frameH: number) {
	const width = p.width * frameW;
	const height = width / shownAspect(p, sourceAspect);
	const left = clamp(p.x * frameW, 0, Math.max(0, frameW - width));
	const top = clamp(p.y * frameH, 0, Math.max(0, frameH - height));
	return { left, top, width, height };
}

/** Corner rounding in pixels for a box this size. */
export function cornerRadius(p: Placement, width: number, height: number): number {
	if (p.shape === 'circle') return Math.min(width, height) / 2;
	if (p.shape === 'rounded') return Math.min(width, height) * 0.12;
	return 0;
}

/** A placement read from a clip's details (`camera` or `logo.placement`), or null. */
export function parsePlacement(json: string | undefined): Placement | null {
	if (!json) return null;
	try {
		const value = JSON.parse(json) as Partial<Placement>;
		return { ...DEFAULT_CAMERA_PLACEMENT, ...value, crop: { ...WHOLE, ...value.crop } };
	} catch {
		return null;
	}
}

/** The logo a clip was saved with, from its details, or null. */
export function parseLogo(json: string | undefined): { path: string; placement: Placement } | null {
	if (!json) return null;
	try {
		const value = JSON.parse(json) as Partial<LogoSettings>;
		if (!value.path) return null;
		const placement = parsePlacement(JSON.stringify(value.placement ?? {})) ?? DEFAULT_LOGO_PLACEMENT;
		return { path: value.path, placement };
	} catch {
		return null;
	}
}

/** Moves a placement by a drag of `dx`, `dy` in fractions of the video, kept on it. */
export function moved(p: Placement, start: Placement, dx: number, dy: number, heightFrac: number): Placement {
	return {
		...p,
		x: clamp(start.x + dx, 0, 1 - p.width),
		y: clamp(start.y + dy, 0, Math.max(0, 1 - heightFrac))
	};
}
