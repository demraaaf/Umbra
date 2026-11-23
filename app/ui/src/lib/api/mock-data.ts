// Deterministic sample data for the in-memory mock (browser / `npm run dev`).
// Nothing here is used when running inside Tauri.

import { DEFAULT_CAMERA_PLACEMENT, DEFAULT_LOGO_PLACEMENT } from '$lib/editor/overlay';
import { DEFAULT_INPUT_OVERLAY, DEFAULT_INPUT_RULES } from '$lib/input/overlay';
import type {
	CameraInfo,
	AudioAppInfo,
	Clip,
	ClipAudioTrack,
	MicrophoneInfo,
	MonitorInfo,
	Settings,
	SystemInfo,
	VideoCodec,
	WindowInfo
} from './types';

// ---------- Seeded random numbers, so screenshots and tests are stable ----------

export function rng(seed: number): () => number {
	let a = seed >>> 0;
	return () => {
		a = (a + 0x6d2b79f5) >>> 0;
		let t = a;
		t = Math.imul(t ^ (t >>> 15), t | 1);
		t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
		return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
	};
}

// ---------- Static machine info ----------

export const MONITORS: MonitorInfo[] = [
	{ device: '\\\\.\\DISPLAY1', width: 2560, height: 1440, refreshHz: 360, primary: true, nativeWidth: 2560, nativeHeight: 1440 },
	{ device: '\\\\.\\DISPLAY2', width: 1920, height: 1080, refreshHz: 144, primary: false, nativeWidth: 1920, nativeHeight: 1080 }
];

export const GAMES = [
	{ name: 'Valorant', exe: 'VALORANT-Win64-Shipping.exe', title: 'VALORANT' },
	{ name: 'Counter-Strike 2', exe: 'cs2.exe', title: 'Counter-Strike 2' },
	{ name: 'Apex Legends', exe: 'r5apex_dx12.exe', title: 'Apex Legends' },
	{ name: 'Rocket League', exe: 'RocketLeague.exe', title: 'Rocket League (64-bit, DX11, Cooked)' }
] as const;

export const WINDOWS: WindowInfo[] = [
	...GAMES.map((g) => ({ exe: g.exe, title: g.title })),
	{ exe: 'chrome.exe', title: 'Inbox - Google Chrome' },
	{ exe: 'Discord.exe', title: 'Discord' },
	{ exe: 'Code.exe', title: 'plan.md - Umbra - Visual Studio Code' },
	{ exe: 'Spotify.exe', title: 'Spotify Premium' },
	{ exe: 'explorer.exe', title: 'Downloads' }
];

export const AUDIO_APPS: AudioAppInfo[] = [
	{ exe: 'Discord.exe', name: 'Discord' },
	{ exe: 'chrome.exe', name: 'Google Chrome' },
	{ exe: 'Spotify.exe', name: 'Spotify' },
	{ exe: 'msedge.exe', name: 'Microsoft Edge' },
	{ exe: 'Teams.exe', name: 'Microsoft Teams' },
	{ exe: 'vlc.exe', name: 'VLC media player' }
];

export const MICROPHONES: MicrophoneInfo[] = [
	{ id: 'mic-yeti', name: 'Microphone (Blue Yeti)', isDefault: true },
	{ id: 'mic-arctis', name: 'Headset Microphone (Arctis Nova Pro)', isDefault: false },
	{ id: 'mic-line', name: 'Line In (Realtek High Definition Audio)', isDefault: false }
];

export const CAMERAS: CameraInfo[] = [
	{
		id: 'cam-brio',
		name: 'Logitech BRIO',
		sizes: [
			{ width: 640, height: 360 },
			{ width: 1280, height: 720 },
			{ width: 1920, height: 1080 },
			{ width: 3840, height: 2160 }
		],
		api: 'Media Foundation'
	},
	{ id: 'dshow:@device_sw_obs', name: 'OBS Virtual Camera', sizes: [], api: 'DirectShow' }
];

export const SYSTEM_INFO: SystemInfo = {
	appVersion: '1.0.0',
	gpu: 'NVIDIA GeForce RTX 5070 Ti',
	codecs: ['h264', 'hevc', 'av1'],
	settingsFolder: 'C:\\Users\\you\\AppData\\Roaming\\Umbra'
};

export const DEFAULT_SETTINGS: Settings = {
	capture: { kind: 'monitor', device: '\\\\.\\DISPLAY1' },
	video: {
		codec: 'hevc',
		width: null,
		height: null,
		fps: 144,
		rateMode: 'cbr',
		bitrateKbps: 60000,
		maxBitrateKbps: 90000,
		qp: 18,
		cq: 23,
		preset: 5,
		captureCursor: true,
		keyframeSecs: 1,
		tune: 'hq',
		multipass: 'off',
		lookahead: 0,
		spatialAq: false,
		temporalAq: false,
		aqStrength: 8,
		scaling: 'stretch', hdr: 'sdr', skipStill: false
	},
	presets: [
		{
			id: 'shooters',
			name: 'FPS',
			games: ['cs2.exe', 'VALORANT-Win64-Shipping.exe', 'r5apex.exe'],
			video: {
				codec: 'hevc', width: null, height: null, fps: 120, rateMode: 'vbr', bitrateKbps: 50000, maxBitrateKbps: 80000,
				qp: 20, cq: 23, preset: 5, captureCursor: true, keyframeSecs: 1, tune: 'hq', multipass: 'off', lookahead: 0,
				spatialAq: true, temporalAq: false, aqStrength: 8, scaling: 'stretch', hdr: 'sdr', skipStill: false
			}
		},
		{
			id: 'story',
			name: 'Story and open world',
			games: ['eldenring.exe', 'Cyberpunk2077.exe'],
			video: {
				codec: 'hevc', width: null, height: null, fps: 60, rateMode: 'vbr', bitrateKbps: 45000, maxBitrateKbps: 70000,
				qp: 20, cq: 23, preset: 6, captureCursor: true, keyframeSecs: 1, tune: 'hq', multipass: 'off', lookahead: 0,
				spatialAq: true, temporalAq: false, aqStrength: 8, scaling: 'stretch', hdr: 'sdr', skipStill: false
			}
		}
	],
	audio: {
		mode: 'split',
		game: true,
		apps: ['Discord.exe'],
		others: true,
		mic: { enabled: true, deviceId: null, pushToTalk: false, pushToTalkKey: 'Mouse5', inMix: false },
		bitrateKbps: 160
	},
	clip: { lengthSecs: 60, hotkey: 'Alt+F10', saveSound: true, saveSoundVolume: 0.6, postSecs: 0, bufferOnDisk: false, bufferFolder: '' },
	screenshot: { hotkey: 'Alt+F11', format: 'png', sound: true },
	recording: {
		mode: 'games',
		launchAtStartup: false,
		startMinimized: true,
		games: ['VALORANT-Win64-Shipping.exe'],
		recordHotkey: 'Alt+F9',
		sessionHotkey: 'Alt+F12',
		fullSessions: false,
		sessionSource: 'capture',
		pauseHotkey: 'Alt+F8',
		neverRecord: ['vlc.exe']
	},
	storage: { folder: 'D:\\Clips', folderTemplate: '{game}', nameTemplate: '{game} {date} {time}', skipRecycleBin: false, limitGb: 0, maxAgeDays: 0, keepAlbums: true, minFreeGb: 5, shrink: { enabled: false, afterDays: 7, quality: 'balanced', includeClips: false, maxFps: 0 } },
	player: { seekSecs: 5 },
	export: { threads: 0, burnReplaceOriginal: false },
	overlay: { enabled: true, corner: 'topRight', status: 'off', statusGames: [], statusCorner: 'topLeft' },
	panel: { widgets: ['status', 'actions', 'source', 'mic', 'usage'] },
	camera: {
		enabled: true,
		deviceId: 'cam-brio',
		deviceName: 'Logitech BRIO',
		width: 1280,
		height: 720,
		placement: { ...DEFAULT_CAMERA_PLACEMENT },
		hotkey: 'Alt+F7'
	},
	logo: { path: null, placement: { ...DEFAULT_LOGO_PLACEMENT } },
	input: {
		enabled: false,
		hotkey: '',
		rules: structuredClone(DEFAULT_INPUT_RULES),
		overlay: structuredClone(DEFAULT_INPUT_OVERLAY)
	},
	autoClip: {
		enabled: false,
		mergeSecs: 10,
		soundVolume: 0.3,
		minKills: 1,
		kill: { enabled: true, minConfidence: 0.5, preSecs: 8, postSecs: 3 },
		headshot: { enabled: false, minConfidence: 0.5, preSecs: 8, postSecs: 3 },
		knife: { enabled: true, minConfidence: 0.5, preSecs: 8, postSecs: 3 },
		zeus: { enabled: true, minConfidence: 0.5, preSecs: 8, postSecs: 3 },
		collateral: { enabled: true, minConfidence: 0.5, preSecs: 8, postSecs: 3 },
		blindKill: { enabled: false, minConfidence: 0.5, preSecs: 8, postSecs: 3 },
		death: { enabled: false, minConfidence: 0.5, preSecs: 8, postSecs: 3 },
		roundWin: { enabled: false, minConfidence: 0.5, preSecs: 8, postSecs: 3 },
		cs2: {
			enabled: false,
			port: 47380,
			token: '3f9a1c0d7be24e51a6c88f0e2d41b7a9',
			modes: ['casual', 'competitive', 'other', 'wingman']
		}
	}
};

// ---------- Thumbnails: abstract greyscale scenes, one style per game ----------

const n = (v: number) => Math.round(v * 10) / 10;
const g = (v: number, a = 1) => `rgba(${Math.round(v)},${Math.round(v)},${Math.round(v)},${a})`;

function scene(kind: string, r: () => number, W: number, H: number): string {
	const horizon = H * (0.5 + r() * 0.12);
	let out = '';
	switch (kind) {
		case 'Valorant': {
			out += `<rect width="${W}" height="${H}" fill="url(#sky)"/>`;
			for (let i = 0; i < 5; i++) {
				const x = r() * W * 0.9;
				const w = 40 + r() * 90;
				const top = horizon - 20 - r() * 70;
				const lean = (r() - 0.5) * 30;
				const shade = 30 + r() * 50;
				out += `<polygon points="${n(x)},${H} ${n(x + lean)},${n(top)} ${n(x + w + lean)},${n(top + 14)} ${n(x + w)},${H}" fill="${g(shade)}"/>`;
				out += `<polyline points="${n(x + lean)},${n(top)} ${n(x + w + lean)},${n(top + 14)}" stroke="${g(200, 0.5)}" stroke-width="1" fill="none"/>`;
			}
			const bx = 40 + r() * 200;
			out += `<polygon points="${n(bx)},0 ${n(bx + 26)},0 ${n(bx + 120)},${H} ${n(bx + 60)},${H}" fill="${g(255, 0.07)}"/>`;
			out += `<rect y="${n(horizon + 10)}" width="${W}" height="${H}" fill="${g(14, 0.55)}"/>`;
			break;
		}
		case 'Counter-Strike 2': {
			out += `<rect width="${W}" height="${H}" fill="${g(38)}"/>`;
			const doorW = 50 + r() * 40;
			const dx = W * (0.3 + r() * 0.35);
			out += `<rect x="0" y="0" width="${n(dx)}" height="${H}" fill="${g(58)}"/>`;
			out += `<polygon points="0,0 ${n(dx)},${n(horizon - 44)} ${n(dx)},${n(horizon + 34)} 0,${H}" fill="${g(46)}"/>`;
			out += `<rect x="${n(dx)}" y="${n(horizon - 44)}" width="${n(doorW)}" height="78" fill="url(#glow)"/>`;
			out += `<rect x="${n(dx + doorW)}" y="0" width="${n(W - dx - doorW)}" height="${H}" fill="${g(70)}"/>`;
			out += `<polygon points="0,${H} ${n(dx)},${n(horizon + 34)} ${n(dx + doorW)},${n(horizon + 34)} ${W},${H}" fill="${g(26)}"/>`;
			for (let i = 0; i < 3; i++) {
				const bw = 22 + r() * 26;
				const bx = r() * (W - bw);
				out += `<rect x="${n(bx)}" y="${n(H - 30 - r() * 30)}" width="${n(bw)}" height="${n(20 + r() * 26)}" fill="${g(84 + r() * 40)}" stroke="${g(0, 0.5)}"/>`;
			}
			break;
		}
		case 'Apex Legends': {
			out += `<rect width="${W}" height="${H}" fill="url(#sky)"/>`;
			const mx = W * (0.2 + r() * 0.6);
			out += `<circle cx="${n(mx)}" cy="${n(horizon - 46)}" r="${n(26 + r() * 14)}" fill="${g(230, 0.32)}"/>`;
			out += `<circle cx="${n(mx)}" cy="${n(horizon - 46)}" r="${n(44 + r() * 18)}" fill="none" stroke="${g(230, 0.16)}"/>`;
			for (let layer = 0; layer < 3; layer++) {
				const base = horizon + layer * 16 - 24;
				let d = `M0 ${H} L0 ${n(base)}`;
				for (let x = 0; x <= W; x += 32) d += ` L${x} ${n(base - r() * (50 - layer * 12))}`;
				d += ` L${W} ${H} Z`;
				out += `<path d="${d}" fill="${g(28 + layer * 22)}"/>`;
			}
			break;
		}
		case 'Rocket League': {
			out += `<rect width="${W}" height="${H}" fill="${g(20)}"/>`;
			out += `<rect x="${W * 0.32}" y="${n(horizon - 46)}" width="${W * 0.36}" height="46" fill="${g(90)}" stroke="${g(210, 0.6)}"/>`;
			out += `<polygon points="0,${H} ${W * 0.3},${n(horizon)} ${W * 0.7},${n(horizon)} ${W},${H}" fill="${g(34)}"/>`;
			for (let i = 1; i < 8; i++) {
				const t = i / 8;
				out += `<line x1="${n(W * (0.3 + 0.4 * t))}" y1="${n(horizon)}" x2="${n(W * (-0.6 + 2.2 * t))}" y2="${H}" stroke="${g(200, 0.16)}"/>`;
				out += `<line x1="0" y1="${n(horizon + (H - horizon) * t * t)}" x2="${W}" y2="${n(horizon + (H - horizon) * t * t)}" stroke="${g(200, 0.12)}"/>`;
			}
			const bx = W * (0.25 + r() * 0.5);
			out += `<ellipse cx="${n(bx)}" cy="${n(H - 20)}" rx="26" ry="5" fill="${g(0, 0.5)}"/>`;
			out += `<circle cx="${n(bx)}" cy="${n(H - 42)}" r="22" fill="${g(210)}"/>`;
			out += `<circle cx="${n(bx - 7)}" cy="${n(H - 49)}" r="7" fill="${g(255, 0.5)}"/>`;
			break;
		}
		default: {
			out += `<rect width="${W}" height="${H}" fill="${g(24)}"/>`;
			for (let i = 0; i < 3; i++) {
				const w = 120 + r() * 100;
				const h = 70 + r() * 50;
				const x = r() * (W - w);
				const y = 8 + r() * (H - h - 16);
				out += `<rect x="${n(x)}" y="${n(y)}" width="${n(w)}" height="${n(h)}" fill="${g(44 + i * 10)}" stroke="${g(120, 0.5)}"/>`;
				out += `<rect x="${n(x)}" y="${n(y)}" width="${n(w)}" height="10" fill="${g(90)}"/>`;
			}
		}
	}
	return out;
}

/** A 16:9 greyscale SVG scene as a data URI. Same seed and game give the same picture. */
export function makeThumbnail(seed: number, game: string | null): string {
	const r = rng(seed * 7919 + 13);
	const W = 320;
	const H = 180;
	const top = 16 + Math.floor(r() * 14);
	const bottom = 46 + Math.floor(r() * 30);
	const svg =
		`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${W} ${H}" width="${W}" height="${H}">` +
		`<defs>` +
		`<linearGradient id="sky" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="${g(top)}"/><stop offset="1" stop-color="${g(bottom)}"/></linearGradient>` +
		`<linearGradient id="glow" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="${g(235)}"/><stop offset="1" stop-color="${g(150)}"/></linearGradient>` +
		`<radialGradient id="vig" cx="0.5" cy="0.5" r="0.75"><stop offset="0.55" stop-color="#000" stop-opacity="0"/><stop offset="1" stop-color="#000" stop-opacity="0.65"/></radialGradient>` +
		`</defs>` +
		scene(game ?? 'Desktop', r, W, H) +
		`<rect width="${W}" height="${H}" fill="url(#vig)"/>` +
		`</svg>`;
	return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}

// ---------- Clips ----------

interface Format {
	width: number;
	height: number;
	fps: number;
	codec: VideoCodec;
	mbps: number;
}

const FORMATS: Format[] = [
	{ width: 2560, height: 1440, fps: 240, codec: 'hevc', mbps: 70 },
	{ width: 2560, height: 1440, fps: 144, codec: 'hevc', mbps: 60 },
	{ width: 2560, height: 1440, fps: 360, codec: 'av1', mbps: 90 },
	{ width: 1920, height: 1080, fps: 144, codec: 'h264', mbps: 40 },
	{ width: 1920, height: 1080, fps: 60, codec: 'h264', mbps: 20 },
	{ width: 1920, height: 1080, fps: 240, codec: 'hevc', mbps: 50 },
	{ width: 3840, height: 2160, fps: 60, codec: 'hevc', mbps: 100 },
	{ width: 1280, height: 720, fps: 120, codec: 'h264', mbps: 16 }
];

const DURATIONS = [12, 15, 20, 30, 30, 45, 60, 60, 60, 90, 120];

// Laid out like the recorder's files: a combined `Mix` first when it combines more than one
// track. The microphone is only on its own track.
const TRACK_SETS: ClipAudioTrack[][] = [
	[
		{ index: 0, title: 'Mix', role: 'mix' },
		{ index: 1, title: 'Game', role: 'inMix' },
		{ index: 2, title: 'Discord', role: 'inMix' },
		{ index: 3, title: 'Other apps', role: 'inMix' },
		{ index: 4, title: 'Microphone', role: 'separate' }
	],
	[
		{ index: 0, title: 'Mix', role: 'mix' },
		{ index: 1, title: 'Game', role: 'inMix' },
		{ index: 2, title: 'Discord', role: 'inMix' },
		{ index: 3, title: 'Microphone', role: 'separate' }
	],
	[
		{ index: 0, title: 'Mix', role: 'mix' },
		{ index: 1, title: 'Game', role: 'inMix' },
		{ index: 2, title: 'Other apps', role: 'inMix' }
	],
	[{ index: 0, title: 'System', role: 'separate' }]
];

const pad = (v: number) => String(v).padStart(2, '0');

export function fileStamp(date: Date): { date: string; time: string } {
	return {
		date: `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`,
		time: `${pad(date.getHours())}-${pad(date.getMinutes())}-${pad(date.getSeconds())}`
	};
}

export function clipPath(folder: string, game: string | null, fileName: string): string {
	return `${folder}\\${game ?? 'Desktop'}\\${fileName}`;
}

export function mockVideoUrl(path: string): string {
	return `mock://clip/${encodeURIComponent(path)}`;
}

/** Builds one clip record; used for the seed data and for clips "saved" while the mock runs. */
export function buildClip(input: {
	seed: number;
	game: string | null;
	createdAt: Date;
	durationSecs: number;
	format: Format;
	tracks: ClipAudioTrack[];
	folder?: string;
	fileName?: string;
	thumbnail?: boolean;
}): Clip {
	const stamp = fileStamp(input.createdAt);
	const fileName = input.fileName ?? `${input.game ?? 'Desktop'} ${stamp.date} ${stamp.time}.mp4`;
	const path = clipPath(input.folder ?? 'D:\\Clips', input.game, fileName);
	const r = rng(input.seed + 101);
	const bytesPerSec = ((input.format.mbps * 1_000_000) / 8) * (0.88 + r() * 0.2);
	const audioBytes = input.tracks.length * (160_000 / 8) * input.durationSecs;
	return {
		path,
		kind: 'video',
		fileName,
		game: input.game,
		createdAt: input.createdAt.toISOString(),
		durationSecs: input.durationSecs,
		sizeBytes: Math.round(bytesPerSec * input.durationSecs + audioBytes),
		width: input.format.width,
		height: input.format.height,
		fps: input.format.fps,
		codec: input.format.codec,
		thumbnailUrl: input.thumbnail === false ? null : makeThumbnail(input.seed, input.game),
		videoUrl: mockVideoUrl(path),
		audioTracks: input.tracks,
		camera: input.seed % 3 === 0 ? { width: 1280, height: 720 } : null,
		input: input.seed % 2 === 0,
		logo: input.seed % 5 === 0,
		albums: [],
		bookmarks: [],
		bitRate: Math.round(bytesPerSec * 8),
		details: {
			game: input.game ?? 'Desktop',
			...(input.game ? { game_exe: `${input.game.replace(/\s+/g, '')}.exe`, detected: 'fullscreen program' } : { detected: 'no game' }),
			source: `Display 1, ${input.format.width}×${input.format.height}`,
			encoder: input.format.codec === 'hevc' ? 'hevc_nvenc' : input.format.codec === 'av1' ? 'av1_nvenc' : 'h264_nvenc',
			gpu: 'NVIDIA GeForce RTX 5070 Ti',
			...(input.seed % 3 === 0 ? { camera: JSON.stringify(DEFAULT_CAMERA_PLACEMENT) } : {}),
			...(input.seed % 2 === 0 ? { input: JSON.stringify(DEFAULT_INPUT_OVERLAY) } : {}),
			rate: `VBR ${Math.round(input.format.mbps * 1000)} kbit/s, up to ${Math.round(input.format.mbps * 1600)} kbit/s`,
			preset: 'P4',
			recorded_fps: String(input.format.fps),
			app: 'Umbra 1.0.0'
		}
	};
}

export function pickFormat(r: () => number): Format {
	return FORMATS[Math.floor(r() * FORMATS.length)];
}

export function pickTracks(r: () => number): ClipAudioTrack[] {
	const roll = r();
	if (roll < 0.5) return TRACK_SETS[0];
	if (roll < 0.72) return TRACK_SETS[1];
	if (roll < 0.9) return TRACK_SETS[2];
	return TRACK_SETS[3];
}

/** 24 clips over the last three weeks: 4 games plus two desktop captures. */
export function seedClips(now = new Date()): Clip[] {
	const r = rng(2026);
	const clips: Clip[] = [];
	let minutesAgo = 22;
	for (let i = 0; i < 24; i++) {
		const game = i === 9 || i === 20 ? null : GAMES[Math.floor(r() * GAMES.length)].name;
		const createdAt = new Date(now.getTime() - minutesAgo * 60_000);
		clips.push(
			buildClip({
				seed: i + 1,
				game,
				createdAt,
				durationSecs: DURATIONS[Math.floor(r() * DURATIONS.length)],
				format: pickFormat(r),
				tracks: pickTracks(r)
			})
		);
		minutesAgo += 40 + Math.floor(r() * 60 * (i < 6 ? 6 : 30));
	}
	return clips;
}

// ---------- Screenshots ----------

/** A screenshot is a `Clip` with `kind: 'image'`: no duration, no audio, an image URL. */
export function buildImage(input: {
	seed: number;
	game: string | null;
	createdAt: Date;
	width: number;
	height: number;
	format: 'png' | 'jpg';
	folder?: string;
	thumbnail?: boolean;
}): Clip {
	const stamp = fileStamp(input.createdAt);
	const fileName = `${input.game ?? 'Desktop'} ${stamp.date} ${stamp.time}.${input.format}`;
	const path = clipPath(input.folder ?? 'D:\Clips', input.game, fileName);
	const image = makeThumbnail(input.seed, input.game);
	const pixels = input.width * input.height;
	return {
		path,
		kind: 'image',
		fileName,
		game: input.game,
		createdAt: input.createdAt.toISOString(),
		durationSecs: 0,
		sizeBytes: Math.round(pixels * (input.format === 'png' ? 1.4 : 0.22)),
		width: input.width,
		height: input.height,
		fps: 0,
		codec: 'h264',
		thumbnailUrl: input.thumbnail === false ? null : image,
		videoUrl: image,
		audioTracks: [],
		albums: [],
		camera: null,
		input: false,
		logo: false,
		bookmarks: [],
		bitRate: 0,
		details: { game: input.game ?? 'Desktop' }
	};
}

/** Five screenshots spread between the seeded clips. */
export function seedScreenshots(now = new Date()): Clip[] {
	const r = rng(77);
	const list: Clip[] = [];
	let minutesAgo = 95;
	for (let i = 0; i < 5; i++) {
		list.push(
			buildImage({
				seed: 300 + i,
				game: GAMES[Math.floor(r() * GAMES.length)].name,
				createdAt: new Date(now.getTime() - minutesAgo * 60_000),
				width: 2560,
				height: 1440,
				format: i % 3 === 0 ? 'jpg' : 'png'
			})
		);
		minutesAgo += 200 + Math.floor(r() * 2400);
	}
	return list;
}

// ---------- Albums ----------

export interface MockAlbum {
	id: string;
	name: string;
	hideFromClips: boolean;
	clipPaths: string[];
	createdAt: string;
	builtin: boolean;
}

/** Favorites, plus two albums over the seeded clips: one visible, one hidden from the Clips screen. */
export function seedAlbums(clips: Clip[], now = new Date()): MockAlbum[] {
	const videos = clips.filter((c) => c.kind === 'video');
	return [
		{
			id: 'favorites',
			name: 'Favorites',
			hideFromClips: false,
			clipPaths: [videos[0], videos[3]].filter(Boolean).map((c) => c.path),
			createdAt: new Date(now.getTime() - 30 * 86_400_000).toISOString(),
			builtin: true
		},
		{
			id: 'album-aces',
			name: 'Aces and clutches',
			hideFromClips: false,
			clipPaths: [videos[1], videos[4], videos[7], videos[12]].filter(Boolean).map((c) => c.path),
			createdAt: new Date(now.getTime() - 6 * 86_400_000).toISOString(),
			builtin: false
		},
		{
			id: 'album-archive',
			name: 'Old scrims',
			hideFromClips: true,
			clipPaths: [videos[17], videos[19], videos[21]].filter(Boolean).map((c) => c.path),
			createdAt: new Date(now.getTime() - 12 * 86_400_000).toISOString(),
			builtin: false
		}
	];
}

// ---------- Capture previews ----------

/** A small grey still standing in for a live preview of a capture source. */
export function makePreview(key: string, tick: number): string {
	let hash = 0;
	for (const ch of key) hash = (hash * 31 + ch.charCodeAt(0)) >>> 0;
	// The seed changes slowly so refreshed previews look alive without flickering.
	return makeThumbnail((hash % 900) + (tick % 3), key.includes('.exe') ? null : 'Valorant');
}
