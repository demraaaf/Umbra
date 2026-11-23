// Export settings: where a clip is going decides how it is written. "Original" changes only
// what was edited; the presets reshape the file for a site; "Custom" sets each part by hand.
// The choice is remembered for the next export.

import type { ExportTarget, OutputFormat, VideoCodec } from '$lib/api/types';
import { formatBytes, formatDuration } from '$lib/format/format';

export type ExportPresetKey = 'original' | 'discord' | 'youtube' | 'phone' | 'custom';

export interface ExportPreset {
	key: ExportPresetKey;
	label: string;
	/** One line on what the preset does. */
	detail: string;
}

export const EXPORT_PRESETS: ExportPreset[] = [
	{ key: 'original', label: 'Original', detail: 'Only your edits. Codec, picture size and frame rate stay as recorded.' },
	{ key: 'discord', label: 'Discord', detail: 'Fits the upload limit: H.264, 60 fps at most, one track with your voice.' },
	{ key: 'youtube', label: 'YouTube', detail: 'Up to 1440p at 60 fps and a high bitrate, one track with your voice.' },
	{ key: 'phone', label: 'Phone', detail: 'Vertical 9:16 from the middle (move the frame to choose), H.264, 60 fps.' },
	{ key: 'custom', label: 'Custom', detail: 'Pick the codec, picture size, frame rate and size yourself.' }
];

export interface FormatChoice {
	value: OutputFormat;
	label: string;
	detail: string;
}

/** Files with a picture. MP4 is what the library shows; the others are saved next to it. */
export const VIDEO_FORMATS: FormatChoice[] = [
	{ value: 'mp4', label: 'MP4', detail: 'Plays everywhere; shown in your library.' },
	{ value: 'mov', label: 'MOV', detail: 'For Final Cut, Premiere and other editors.' },
	{ value: 'mkv', label: 'MKV', detail: 'An open container many players take.' },
	{ value: 'webm', label: 'WebM', detail: 'For the web: AV1 (or VP9) with Opus audio.' },
	{ value: 'gif', label: 'GIF', detail: 'A silent loop for chats. Files get big quickly; keep it short or narrow.' }
];

/** The sound alone. */
export const AUDIO_FORMATS: FormatChoice[] = [
	{ value: 'mp3', label: 'MP3', detail: 'Plays everywhere.' },
	{ value: 'm4a', label: 'M4A', detail: 'AAC; smaller than MP3 at the same quality.' },
	{ value: 'wav', label: 'WAV', detail: 'Uncompressed, for editing.' },
	{ value: 'flac', label: 'FLAC', detail: 'Lossless and smaller than WAV.' },
	{ value: 'opus', label: 'Opus', detail: 'Smallest at the same quality; newer players.' }
];

export function isAudioOnly(format: OutputFormat): boolean {
	return AUDIO_FORMATS.some((f) => f.value === format);
}

/** GIF widths offered; the height follows the picture. */
export const GIF_WIDTHS = [320, 480, 640] as const;

/** Discord's upload limits: free, Nitro Basic, Nitro. */
export const DISCORD_SIZES_MB = [10, 50, 500] as const;
export const DISCORD_TIER: Record<number, string> = { 10: 'Free', 50: 'Nitro Basic', 500: 'Nitro' };

/** Picture heights offered for Custom; 0 keeps the source size. */
export const HEIGHTS = [0, 2160, 1440, 1080, 720, 480] as const;
/** Frame rates offered for Custom; 0 keeps the source rate. */
export const FRAME_RATES = [0, 144, 120, 60, 30] as const;

export interface CustomExport {
	/** `same` keeps the source codec. */
	codec: VideoCodec | 'same';
	/** At most this tall (the width follows); 0 keeps the size. */
	height: number;
	/** At most this many frames per second; 0 keeps the rate. */
	fps: number;
	/** How big: like the source, a set bitrate, or a file size limit. */
	size: 'same' | 'bitrate' | 'limit';
	kbps: number;
	limitMb: number;
}

export interface ExportOptions {
	preset: ExportPresetKey;
	discordMb: number;
	custom: CustomExport;
	/** Original and Custom: keep the tracks apart, or mix them into one (with the microphone). */
	audio: 'tracks' | 'mix';
	/** Put the finished file on the clipboard, ready to paste. */
	copyAfter: boolean;
	format: OutputFormat;
	gifWidth: number;
	/** Cut on the exact frame, re-encoding the video; otherwise on the nearest keyframe, copied. */
	exactCuts: boolean;
}

export const DEFAULT_EXPORT: ExportOptions = {
	preset: 'original',
	discordMb: 10,
	custom: { codec: 'same', height: 0, fps: 0, size: 'same', kbps: 20_000, limitMb: 25 },
	audio: 'tracks',
	copyAfter: false,
	format: 'mp4',
	gifWidth: 480,
	exactCuts: false
};

const STORE_KEY = 'umbra.export';

/** The settings used last time; the defaults the first time or when storage is unavailable. */
export function loadExportOptions(): ExportOptions {
	try {
		const raw = localStorage.getItem(STORE_KEY);
		if (!raw) return structuredClone(DEFAULT_EXPORT);
		const saved = JSON.parse(raw) as Partial<ExportOptions>;
		return {
			...structuredClone(DEFAULT_EXPORT),
			...saved,
			custom: { ...DEFAULT_EXPORT.custom, ...(saved.custom ?? {}) }
		};
	} catch {
		return structuredClone(DEFAULT_EXPORT);
	}
}

export function saveExportOptions(options: ExportOptions) {
	try {
		localStorage.setItem(STORE_KEY, JSON.stringify(options));
	} catch {
		// Remembering is a convenience; the export works without it.
	}
}

/** True when the preset writes one mixed track whatever the audio choice says. */
export function forcesMix(key: ExportPresetKey, format: OutputFormat = 'mp4'): boolean {
	// A file of several clips, a GIF or audio alone has one track (or none) anyway.
	return key === 'discord' || key === 'youtube' || key === 'phone' || format !== 'mp4';
}

/** What the options ask of the file, or null when it is written like the original. */
export function exportTarget(options: ExportOptions): ExportTarget | null {
	const mixOnly = forcesMix(options.preset, options.format) || options.audio === 'mix';
	switch (options.preset) {
		case 'discord':
			return { codec: 'h264', maxBytes: options.discordMb * 1_000_000, maxFps: 60, mixOnly: true };
		case 'youtube':
			return { maxHeight: 1440, maxFps: 60, videoKbps: 40_000, mixOnly: true };
		case 'phone':
			return { codec: 'h264', maxHeight: 1920, maxFps: 60, videoKbps: 15_000, mixOnly: true };
		case 'custom': {
			const c = options.custom;
			const target: ExportTarget = { mixOnly };
			if (c.codec !== 'same') target.codec = c.codec;
			if (c.height > 0) target.maxHeight = c.height;
			if (c.fps > 0) target.maxFps = c.fps;
			if (c.size === 'bitrate' && c.kbps > 0) target.videoKbps = Math.round(c.kbps);
			if (c.size === 'limit' && c.limitMb > 0) target.maxBytes = Math.round(c.limitMb * 1_000_000);
			const changes = Object.keys(target).length > 1 || mixOnly;
			return changes ? target : null;
		}
		default:
			return mixOnly ? { mixOnly: true } : null;
	}
}

/** A short name for the chosen settings, for the button that opens them. */
export function exportLabel(options: ExportOptions): string {
	const format = [...VIDEO_FORMATS, ...AUDIO_FORMATS].find((f) => f.value === options.format)?.label ?? 'MP4';
	if (options.format === 'gif') return `GIF · ${options.gifWidth} px`;
	if (isAudioOnly(options.format)) return `${format} · audio`;
	const preset = presetLabel(options);
	return options.format === 'mp4' ? preset : `${preset} · ${format}`;
}

function presetLabel(options: ExportOptions): string {
	switch (options.preset) {
		case 'discord':
			return `Discord · ${options.discordMb} MB`;
		case 'original':
			return options.audio === 'mix' ? 'Original · 1 track' : 'Original';
		default:
			return EXPORT_PRESETS.find((p) => p.key === options.preset)?.label ?? 'Original';
	}
}

/** One line on what the export will be: length, picture, frame rate, codec, tracks, size. */
export function exportSummary(info: {
	secs: number;
	width: number;
	height: number;
	fps: number;
	codec: string;
	tracks: number;
	bytes: number;
	limit: boolean;
}): string {
	const tracks = info.tracks === 0 ? 'no audio' : info.tracks === 1 ? '1 track' : `${info.tracks} tracks`;
	const size = info.limit ? `under ${formatBytes(info.bytes)}` : `about ${formatBytes(info.bytes)}`;
	if (info.width === 0) return [formatDuration(info.secs), info.codec, size].join(' · ');
	if (info.tracks < 0) return [formatDuration(info.secs), `${info.width}×${info.height}`, `${Math.round(info.fps)} fps`, 'GIF', size].join(' · ');
	return [formatDuration(info.secs), `${info.width}×${info.height}`, `${Math.round(info.fps)} fps`, info.codec, tracks, size].join(' · ');
}
