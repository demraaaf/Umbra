// The keyboard and mouse overlay, drawn on a canvas over the player and the editor. Export
// draws the same in Rust (`core/src/inputdraw.rs`): both take their geometry from
// `layouts.json` and work out what shows at a moment the same way, so keep them in step.

import type { InputBlock, InputOverlay, InputRule, InputTrack, Placement } from '$lib/api/types';
import { DEFAULT_CAMERA_PLACEMENT } from '$lib/editor/overlay';
import layouts from './layouts.json';

type Rect = { x: number; y: number; w: number; h: number };
type LayoutKey = Rect & { label: string; vk: number; scan: number; ext: boolean };
type KeysLayout = { width: number; height: number; keys: LayoutKey[]; text: Rect };
type MouseLayout = (typeof layouts)['mouse'];

/** Bottom left, WASD and the mouse (as `InputOverlayPrefs::default` in Rust). */
export const DEFAULT_INPUT_OVERLAY: InputOverlay = {
	keys: 'wasd',
	mouse: true,
	movement: true,
	ticker: false,
	placement: { ...DEFAULT_CAMERA_PLACEMENT, x: 0.02, y: 0.74, width: 0.2, shape: 'rectangle' } as Placement,
	opacity: 0.9,
	highlight: '#ffffff'
};

/** The rules a new install starts with (as `default_input_rules` in Rust). */
export const DEFAULT_INPUT_RULES: InputRule[] = [
	{ name: 'Games', matches: 'game', apps: [], keyboard: 'full', mouse: 'full', chatKeys: ['Enter'] },
	{
		name: 'Chat apps',
		matches: 'apps',
		apps: ['Discord.exe', 'Telegram.exe', 'WhatsApp.exe', 'Signal.exe', 'ms-teams.exe', 'Teams.exe', 'slack.exe'],
		keyboard: 'drop',
		mouse: 'full',
		chatKeys: []
	},
	{
		name: 'Password managers',
		matches: 'apps',
		apps: ['1Password.exe', 'Bitwarden.exe', 'KeePass.exe', 'KeePassXC.exe', 'Dashlane.exe'],
		keyboard: 'drop',
		mouse: 'drop',
		chatKeys: []
	},
	{ name: 'Browsers', matches: 'apps', apps: ['chrome.exe', 'msedge.exe', 'firefox.exe', 'brave.exe', 'opera.exe'], keyboard: 'redacted', mouse: 'full', chatKeys: [] },
	{ name: 'Everything else', matches: 'any', apps: [], keyboard: 'redacted', mouse: 'full', chatKeys: [] }
];

/** A clip's overlay from its `details.input`, or the settings' look when it has none. The
 * fallback may be reactive state, which `structuredClone` refuses; a JSON copy takes it. */
export function parseInputOverlay(json: string | undefined, fallback: InputOverlay): InputOverlay {
	const base = JSON.parse(JSON.stringify(fallback)) as InputOverlay;
	if (!json) return base;
	try {
		return { ...base, ...(JSON.parse(json) as Partial<InputOverlay>) };
	} catch {
		return base;
	}
}

/** What the overlay is made of, side by side (keys, then the mouse), the ticker under them. */
export interface Composition {
	keys: KeysLayout | null;
	mouse: { x: number; layout: MouseLayout } | null;
	ticker: number | null;
	width: number;
	height: number;
}

export function compose(prefs: InputOverlay): Composition | null {
	const keys: KeysLayout | null = prefs.keys === 'wasd' ? layouts.wasd : prefs.keys === 'keyboard' ? layouts.keyboard : null;
	let width = keys?.width ?? 0;
	let height = keys?.height ?? 0;
	let mouse: Composition['mouse'] = null;
	if (prefs.mouse) {
		const x = keys ? width + layouts.spacing : 0;
		width = x + layouts.mouse.width;
		height = Math.max(height, layouts.mouse.height);
		mouse = { x, layout: layouts.mouse };
	}
	let ticker: number | null = null;
	if (prefs.ticker) {
		ticker = height > 0 ? height + layouts.spacing : 0;
		height = ticker + layouts.ticker.height;
		width = Math.max(width, 6);
	}
	return width > 0 ? { keys, mouse, ticker, width, height } : null;
}

/** What shows at a moment (as `Visual` in Rust). */
export interface Visual {
	keys: boolean[];
	text: boolean;
	buttons: number;
	wheel: number;
	movement: [number, number];
	ticker: string[];
}

const WHEEL_SECS = 0.15;
const MOVE_SECS = 0.1;
const MOVE_FULL = 300;
const TICKER_SECS = 2;

type Held = { keys: [number, number, number][]; text: number; buttons: number };
type InputEvent = [number, number, number, number, number];

const isModifier = (vk: number) => (vk >= 0x10 && vk <= 0x12) || (vk >= 0xa0 && vk <= 0xa5) || vk === 0x5b || vk === 0x5c;

function matches(k: LayoutKey, [vk, scan, ext]: [number, number, number]) {
	return (scan !== 0 && scan === k.scan && !!ext === k.ext) || vk === k.vk;
}

function apply(held: Held, [, kind, a, b, c]: InputEvent) {
	const same = (k: [number, number, number]) => k[0] === a && k[1] === b && k[2] === c;
	if (kind === 1 && !held.keys.some(same)) held.keys.push([a, b, c]);
	else if (kind === 2) held.keys = held.keys.filter((k) => !same(k));
	else if (kind === 3) held.text += 1;
	else if (kind === 4) held.text = Math.max(0, held.text - 1);
	else if (kind === 5) held.buttons |= 1 << a;
	else if (kind === 6) held.buttons &= ~(1 << a);
}

function keyName(vk: number): string {
	return (layouts.names as Record<string, string>)[String(vk)] ?? `#${vk}`;
}

/** A clip's input, ready to tell what shows at any moment. */
export class InputTimeline {
	readonly blocks: InputBlock[];
	readonly events: InputEvent[];

	constructor(track: InputTrack) {
		this.blocks = [...track.blocks].sort((a, b) => a.start - b.start);
		this.events = this.blocks.flatMap((b) => b.events).sort((a, b) => a[0] - b[0]);
	}

	get empty(): boolean {
		return this.blocks.length === 0;
	}

	/** Index of the first item whose time is above `t`. */
	#after<T>(list: T[], time: (item: T) => number, t: number): number {
		let lo = 0;
		let hi = list.length;
		while (lo < hi) {
			const mid = (lo + hi) >> 1;
			if (time(list[mid]) <= t) lo = mid + 1;
			else hi = mid;
		}
		return lo;
	}

	held(t: number): Held {
		const index = this.#after(this.blocks, (b) => b.start, t) - 1;
		if (index < 0) return { keys: [], text: 0, buttons: 0 };
		const block = this.blocks[index];
		const held: Held = { keys: block.keys.map((k) => [...k] as [number, number, number]), text: block.text, buttons: block.buttons };
		for (const event of block.events) {
			if (event[0] > t) break;
			apply(held, event);
		}
		return held;
	}

	#between(from: number, to: number): InputEvent[] {
		const t = (e: InputEvent) => e[0];
		return this.events.slice(this.#after(this.events, t, from), this.#after(this.events, t, to));
	}

	visual(t: number, composition: Composition): Visual {
		const held = this.held(t);
		const keys = composition.keys?.keys.map((k) => held.keys.some((key) => matches(k, key))) ?? [];
		let wheel = 0;
		let dx = 0;
		let dy = 0;
		if (composition.mouse) {
			for (const [, kind, a] of this.#between(t - WHEEL_SECS, t)) if (kind === 7) wheel = Math.sign(a);
			for (const [, kind, a, b] of this.#between(t - MOVE_SECS, t)) {
				if (kind === 8) {
					dx += a;
					dy += b;
				}
			}
		}
		const length = Math.hypot(dx, dy);
		const reach = length < 1 ? 0 : (Math.min(1, length / MOVE_FULL) * 8) / length;
		const ticker: string[] = [];
		if (composition.ticker !== null) {
			for (const event of this.#between(t - TICKER_SECS, t)) {
				const [at, kind, vk] = event;
				if (kind === 3) ticker.push('•');
				else if (kind === 1 && !isModifier(vk)) ticker.push(this.#combo(this.held(at - 1e-6), vk));
			}
		}
		return {
			keys,
			text: held.text > 0,
			buttons: held.buttons,
			wheel,
			movement: [Math.round(dx * reach), Math.round(dy * reach)],
			ticker: ticker.slice(-layouts.ticker.chips)
		};
	}

	#combo(held: Held, vk: number): string {
		const has = (codes: number[]) => held.keys.some(([k]) => codes.includes(k));
		const parts: string[] = [];
		if (has([0x11, 0xa2, 0xa3])) parts.push('Ctrl');
		if (has([0x10, 0xa0, 0xa1])) parts.push('Shift');
		if (has([0x12, 0xa4, 0xa5])) parts.push('Alt');
		if (has([0x5b, 0x5c])) parts.push('Win');
		parts.push(keyName(vk));
		return parts.join('+');
	}
}

const FILL = 'rgba(24, 24, 27, 0.82)';
const EDGE = 'rgba(255, 255, 255, 0.14)';
const LABEL = '#d4d4d8';
/** The mouse wheel's slot, and the arrow's rim. */
const SLOT = 'rgba(0, 0, 0, 0.5)';
/** The wheel, the side buttons and the resting movement dot, when not lit. */
const WHEEL = 'rgba(255, 255, 255, 0.22)';

function lightness(css: string): number {
	const hex = css.replace('#', '');
	const full = hex.length === 3 ? [...hex].map((c) => c + c).join('') : hex;
	const n = parseInt(full, 16) || 0xffffff;
	return (0.2126 * ((n >> 16) & 255) + 0.7152 * ((n >> 8) & 255) + 0.0722 * (n & 255)) / 255;
}

/** Draws `visual` filling `width`×`height` CSS pixels of `ctx` (already scaled for the screen). */
export function drawOverlay(ctx: CanvasRenderingContext2D, composition: Composition, visual: Visual, width: number, prefs: InputOverlay) {
	const unit = width / composition.width;
	const height = composition.height * unit;
	const gap = layouts.gap;
	const radius = layouts.radius * unit;
	const heldLabel = lightness(prefs.highlight) > 0.5 ? '#18181b' : '#ffffff';
	const fontPx = layouts.font * unit;
	ctx.clearRect(0, 0, width, height);
	ctx.save();
	ctx.globalAlpha = Math.min(1, Math.max(0, prefs.opacity));
	ctx.textAlign = 'center';
	ctx.textBaseline = 'middle';
	const font = (px: number) => `600 ${px}px "Segoe UI", system-ui, sans-serif`;
	const box = (r: Rect, dx = 0) => ({
		x: (dx + r.x + gap / 2) * unit,
		y: (r.y + gap / 2) * unit,
		w: (r.w - gap) * unit,
		h: (r.h - gap) * unit
	});
	const shape = (b: { x: number; y: number; w: number; h: number }, lit: boolean, rounding = radius) => {
		ctx.beginPath();
		ctx.roundRect(b.x, b.y, b.w, b.h, rounding);
		ctx.fillStyle = lit ? prefs.highlight : FILL;
		ctx.fill();
		ctx.lineWidth = Math.max(1, 0.03 * unit);
		ctx.strokeStyle = EDGE;
		ctx.stroke();
	};
	const label = (text: string, b: { x: number; y: number; w: number; h: number }, lit: boolean, px: number) => {
		ctx.font = font(px);
		ctx.fillStyle = lit ? heldLabel : LABEL;
		ctx.fillText(text, b.x + b.w / 2, b.y + b.h / 2);
	};

	if (composition.keys) {
		composition.keys.keys.forEach((k, i) => {
			const b = box(k);
			shape(b, visual.keys[i]);
			label(k.label, b, visual.keys[i], [...k.label].length > 2 ? fontPx * 0.7 : fontPx);
		});
		const t = box(composition.keys.text);
		shape(t, visual.text, t.h / 2);
		label('•', t, visual.text, fontPx);
	}
	if (composition.mouse) {
		drawMouse(ctx, composition.mouse.x, composition.mouse.layout, visual, unit, prefs);
	}
	if (composition.ticker !== null) {
		const px = fontPx * 0.95;
		ctx.font = font(px);
		const pad = 0.3 * unit;
		let right = width;
		const y = composition.ticker * unit;
		const h = layouts.ticker.height * unit;
		for (const text of [...visual.ticker].reverse()) {
			const w = ctx.measureText(text).width + 2 * pad;
			if (right - w < 0) break;
			const b = { x: right - w, y, w, h };
			shape(b, false);
			label(text, b, false, px);
			ctx.font = font(px);
			right -= w + 0.15 * unit;
		}
	}
	ctx.restore();
}

/**
 * The mouse (as `Painter::paint` in Rust): an egg-shaped body whose top half is the two
 * buttons, filled when held; the wheel in a slot between them, lit for a middle click, with
 * an arrow the way it turns; the side buttons on the left flank; movement in the palm.
 */
function drawMouse(ctx: CanvasRenderingContext2D, x0: number, m: MouseLayout, visual: Visual, unit: number, prefs: InputOverlay) {
	const u = (v: number) => v * unit;
	const left = u(x0 + m.body.x);
	const right = u(x0 + m.body.x + m.body.w);
	const top = u(m.body.y);
	const bottom = u(m.body.y + m.body.h);
	const cx = (left + right) / 2;
	const rx = (right - left) / 2;
	const rt = u(m.top);
	const rb = u(m.bottom);
	const split = u(m.split);
	const line = Math.max(1, 0.035 * unit);
	const lit = (bit: number) => (visual.buttons & (1 << bit)) !== 0;

	const body = new Path2D();
	body.ellipse(cx, top + rt, rx, rt, 0, Math.PI, 2 * Math.PI);
	body.ellipse(cx, bottom - rb, rx, rb, 0, 0, Math.PI);
	body.closePath();
	ctx.fillStyle = FILL;
	ctx.fill(body);
	// A held button: its half of the top, up to the body's edge.
	const half = (side: -1 | 1) => {
		const p = new Path2D();
		p.moveTo(cx, split);
		p.lineTo(cx, top);
		p.ellipse(cx, top + rt, rx, rt, 0, -Math.PI / 2, side < 0 ? Math.PI : 0, side < 0);
		p.lineTo(cx + side * rx, split);
		p.closePath();
		return p;
	};
	ctx.fillStyle = prefs.highlight;
	if (lit(0)) ctx.fill(half(-1));
	if (lit(1)) ctx.fill(half(1));
	const w = { x: u(x0 + m.wheel.x), y: u(m.wheel.y), w: u(m.wheel.w), h: u(m.wheel.h) };
	const pad = u(0.07);
	// The seams: across under the buttons, and between them, broken by the wheel's slot.
	ctx.strokeStyle = EDGE;
	ctx.lineWidth = line;
	ctx.beginPath();
	ctx.moveTo(left, split);
	ctx.lineTo(right, split);
	ctx.moveTo(cx, top);
	ctx.lineTo(cx, w.y - pad);
	ctx.moveTo(cx, w.y + w.h + pad);
	ctx.lineTo(cx, split);
	ctx.stroke();
	ctx.stroke(body);

	ctx.beginPath();
	ctx.roundRect(w.x - pad, w.y - pad, w.w + 2 * pad, w.h + 2 * pad, (w.w + 2 * pad) / 2);
	ctx.fillStyle = SLOT;
	ctx.fill();
	ctx.beginPath();
	ctx.roundRect(w.x, w.y, w.w, w.h, w.w / 2);
	ctx.fillStyle = lit(2) ? prefs.highlight : WHEEL;
	ctx.fill();
	if (visual.wheel !== 0) {
		// Up above the wheel, down below it, with a dark rim so it shows on a held button too.
		const up = visual.wheel > 0;
		const s = u(0.15);
		const tip = up ? w.y - pad - u(0.1) - s * 0.7 : w.y + w.h + pad + u(0.1) + s * 0.7;
		const arms = up ? tip + s * 0.7 : tip - s * 0.7;
		ctx.beginPath();
		ctx.moveTo(cx - s, arms);
		ctx.lineTo(cx, tip);
		ctx.lineTo(cx + s, arms);
		ctx.lineCap = 'round';
		ctx.lineJoin = 'round';
		ctx.strokeStyle = SLOT;
		ctx.lineWidth = u(0.13);
		ctx.stroke();
		ctx.strokeStyle = prefs.highlight;
		ctx.lineWidth = u(0.07);
		ctx.stroke();
	}

	for (const [r, bit] of [
		[m.x1, 3],
		[m.x2, 4]
	] as const) {
		ctx.beginPath();
		ctx.roundRect(u(x0 + r.x), u(r.y), u(r.w), u(r.h), u(r.w) / 2);
		ctx.fillStyle = lit(bit) ? prefs.highlight : WHEEL;
		ctx.fill();
	}

	if (prefs.movement) {
		const c = m.movement;
		const mx = u(x0 + c.x);
		const my = u(c.y);
		ctx.beginPath();
		ctx.arc(mx, my, u(c.r), 0, Math.PI * 2);
		ctx.strokeStyle = EDGE;
		ctx.lineWidth = line;
		ctx.stroke();
		const [dx, dy] = visual.movement;
		if (dx === 0 && dy === 0) {
			ctx.beginPath();
			ctx.arc(mx, my, u(0.08), 0, Math.PI * 2);
			ctx.fillStyle = WHEEL;
			ctx.fill();
		} else {
			const ex = mx + (u(c.r) * 0.78 * dx) / 8;
			const ey = my + (u(c.r) * 0.78 * dy) / 8;
			ctx.beginPath();
			ctx.moveTo(mx, my);
			ctx.lineTo(ex, ey);
			ctx.lineCap = 'round';
			ctx.strokeStyle = prefs.highlight;
			ctx.lineWidth = u(0.07);
			ctx.stroke();
			ctx.beginPath();
			ctx.arc(ex, ey, u(0.13), 0, Math.PI * 2);
			ctx.fillStyle = prefs.highlight;
			ctx.fill();
		}
	}
}

/** Made-up input for the mock backend: WASD, clicks, a little typing and movement. */
export function mockInputTrack(durationSecs: number): InputTrack {
	const keys: [string, number, number][] = [
		['W', 0x57, 0x11],
		['A', 0x41, 0x1e],
		['S', 0x53, 0x1f],
		['D', 0x44, 0x20]
	];
	const blocks: InputBlock[] = [];
	for (let start = 0; start < durationSecs; start += 1) {
		const events: InputEvent[] = [];
		const [, vk, scan] = keys[start % keys.length];
		events.push([start + 0.1, 1, vk, scan, 0], [start + 0.6, 2, vk, scan, 0]);
		if (start % 3 === 0) events.push([start + 0.3, 5, 0, 0, 0], [start + 0.45, 6, 0, 0, 0]);
		if (start % 7 === 4) events.push([start + 0.2, 3, 0, 0, 0], [start + 0.3, 4, 0, 0, 0]);
		if (start % 5 === 2) events.push([start + 0.7, 7, start % 2 ? 120 : -120, 0, 0]);
		for (let i = 0; i < 30; i++) {
			const t = start + i / 30;
			events.push([t, 8, Math.round(Math.sin(t * 2) * 40), Math.round(Math.cos(t * 3) * 25), 0]);
		}
		events.sort((a, b) => a[0] - b[0]);
		blocks.push({ start, keys: [], text: 0, buttons: 0, events });
	}
	return { blocks };
}
