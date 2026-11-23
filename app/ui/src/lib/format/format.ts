// Small, pure formatting helpers. Timecodes and sizes are always tabular-friendly strings.

import type { Clip } from '$lib/api/types';

const pad2 = (v: number) => String(Math.floor(v)).padStart(2, '0');

/** `0:47`, `12:05`, `1:02:03`. For badges and lists, whole seconds. */
/** How a video was made: a clip, or a recording (`manual`) or full game session (`session`). */
export function recordingKind(clip: Pick<Clip, 'details'>): 'manual' | 'session' | null {
	const kind = clip.details?.recording;
	return kind === 'manual' || kind === 'session' ? kind : null;
}

export function formatDuration(secs: number): string {
	const total = Math.max(0, Math.round(secs));
	const h = Math.floor(total / 3600);
	const m = Math.floor((total % 3600) / 60);
	const s = total % 60;
	return h > 0 ? `${h}:${pad2(m)}:${pad2(s)}` : `${m}:${pad2(s)}`;
}

/** `0:12.417`, `1:02:03.500`. Millisecond precision for the editor. */
export function formatTimecode(secs: number): string {
	const clamped = Math.max(0, secs);
	const totalMs = Math.round(clamped * 1000);
	const ms = totalMs % 1000;
	const total = Math.floor(totalMs / 1000);
	const h = Math.floor(total / 3600);
	const m = Math.floor((total % 3600) / 60);
	const s = total % 60;
	const tail = `${pad2(s)}.${String(ms).padStart(3, '0')}`;
	return h > 0 ? `${h}:${pad2(m)}:${tail}` : `${m}:${tail}`;
}

/** Accepts `12`, `12.5`, `1:02`, `1:02.5`, `1:02:03.250`. Returns null if it cannot be read. */
export function parseTimecode(input: string): number | null {
	const text = input.trim().replace(',', '.');
	if (!text) return null;
	const parts = text.split(':');
	if (parts.length > 3) return null;
	let seconds = 0;
	for (const part of parts) {
		if (!/^\d+(\.\d+)?$/.test(part)) return null;
		seconds = seconds * 60 + parseFloat(part);
	}
	return Number.isFinite(seconds) ? seconds : null;
}

const UNITS = ['B', 'KB', 'MB', 'GB', 'TB'];

/** `384 MB`, `1.2 GB`. Decimal units, matching how Windows Explorer reports sizes. */
export function formatBytes(bytes: number): string {
	if (!Number.isFinite(bytes) || bytes <= 0) return '0 B';
	let value = bytes;
	let unit = 0;
	while (value >= 1000 && unit < UNITS.length - 1) {
		value /= 1000;
		unit++;
	}
	const digits = value >= 100 || unit === 0 ? 0 : 1;
	return `${value.toFixed(digits)} ${UNITS[unit]}`;
}

/** `60 Mbit/s`, `800 kbit/s`. */
export function formatBitrate(kbps: number): string {
	if (kbps >= 1000) {
		const mbps = kbps / 1000;
		return `${Number.isInteger(mbps) ? mbps : mbps.toFixed(1)} Mbit/s`;
	}
	return `${kbps} kbit/s`;
}

export function formatResolution(width: number, height: number): string {
	return `${width}×${height}`;
}

/** Whole numbers stay whole (60); fractional rates keep up to two decimals (59.94). */
export function formatFps(fps: number): string {
	return String(Number(fps.toFixed(2)));
}

export function formatFormat(clip: Pick<Clip, 'width' | 'height' | 'fps'>): string {
	return `${formatResolution(clip.width, clip.height)} · ${formatFps(clip.fps)} fps`;
}

export const CODEC_LABELS = { h264: 'H.264', hevc: 'HEVC', av1: 'AV1' } as const;

/** File name without the extension, which is what people read as the clip's name. */
export function clipTitle(clip: Pick<Clip, 'fileName'>): string {
	return clip.fileName.replace(/\.[^.\\/]+$/, '');
}

export function gameLabel(game: string | null): string {
	return game ?? 'Desktop';
}

const dayMs = 86_400_000;

/** `Today 21:14`, `Yesterday 18:02`, `Mon 14:20`, `12 Sep`. */
export function formatWhen(iso: string, now = new Date()): string {
	const date = new Date(iso);
	if (Number.isNaN(date.getTime())) return '';
	const time = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit', hour12: false }).format(date);
	const startOfDay = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
	const diffDays = Math.round((startOfDay(now) - startOfDay(date)) / dayMs);
	if (diffDays === 0) return `Today ${time}`;
	if (diffDays === 1) return `Yesterday ${time}`;
	if (diffDays > 1 && diffDays < 7) {
		return `${new Intl.DateTimeFormat(undefined, { weekday: 'short' }).format(date)} ${time}`;
	}
	const sameYear = date.getFullYear() === now.getFullYear();
	return new Intl.DateTimeFormat(undefined, {
		day: 'numeric',
		month: 'short',
		...(sameYear ? {} : { year: 'numeric' })
	}).format(date);
}

/** Full local date and time for tooltips. */
export function formatFullDate(iso: string): string {
	const date = new Date(iso);
	if (Number.isNaN(date.getTime())) return '';
	return new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'medium', hour12: false }).format(date);
}

export function plural(count: number, one: string, other = `${one}s`): string {
	return `${count} ${count === 1 ? one : other}`;
}

export function clamp(value: number, min: number, max: number): number {
	return Math.min(max, Math.max(min, value));
}
