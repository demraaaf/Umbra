// What the recording panel on the right can show. The layout is a list of widget ids in the
// settings (`panel.widgets`): the panel's own parts, and `setting:<key>` for any setting below,
// which changes that setting right there. Unknown ids (from a newer or older Umbra) are skipped.

import type { Component } from 'svelte';
import type { Settings } from '$lib/api/types';
import ActivityIcon from '@lucide/svelte/icons/activity';
import AppWindowIcon from '@lucide/svelte/icons/app-window';
import BellIcon from '@lucide/svelte/icons/bell';
import CameraIcon from '@lucide/svelte/icons/camera';
import VideoIcon from '@lucide/svelte/icons/video';
import ClapperboardIcon from '@lucide/svelte/icons/clapperboard';
import CpuIcon from '@lucide/svelte/icons/cpu';
import DatabaseIcon from '@lucide/svelte/icons/database';
import FilmIcon from '@lucide/svelte/icons/film';
import Gamepad2Icon from '@lucide/svelte/icons/gamepad-2';
import HardDriveIcon from '@lucide/svelte/icons/hard-drive';
import KeyboardIcon from '@lucide/svelte/icons/keyboard';
import LayersIcon from '@lucide/svelte/icons/layers';
import MicIcon from '@lucide/svelte/icons/mic';
import MonitorIcon from '@lucide/svelte/icons/monitor';
import MousePointerIcon from '@lucide/svelte/icons/mouse-pointer-2';
import PowerIcon from '@lucide/svelte/icons/power';
import RadioIcon from '@lucide/svelte/icons/radio';
import Trash2Icon from '@lucide/svelte/icons/trash-2';
import Volume2Icon from '@lucide/svelte/icons/volume-2';
import AppWindowMacIcon from '@lucide/svelte/icons/app-window-mac';

export const DEFAULT_PANEL = ['status', 'actions', 'source', 'mic', 'usage'];

/** One of the panel's own parts. */
export interface BlockWidget {
	kind: 'block';
	id: string;
	title: string;
	description: string;
	icon: Component;
}

export const BLOCKS: BlockWidget[] = [
	{ kind: 'block', id: 'status', title: 'Replay buffer', description: 'On or off, how full it is, the picture it records.', icon: RadioIcon },
	{ kind: 'block', id: 'actions', title: 'Save, screenshot, record', description: 'The buttons, with their hotkeys, and the running recording.', icon: ClapperboardIcon },
	{ kind: 'block', id: 'source', title: 'Capture source', description: 'Switch between screens and windows.', icon: MonitorIcon },
	{ kind: 'block', id: 'mic', title: 'Microphone', description: 'On or off, push to talk, which device.', icon: MicIcon },
	{ kind: 'block', id: 'camera', title: 'Camera', description: 'On or off, and whether it is recording.', icon: VideoIcon },
	{ kind: 'block', id: 'input', title: 'Keyboard and mouse', description: 'On or off, and what the program in front keeps.', icon: KeyboardIcon },
	{ kind: 'block', id: 'usage', title: 'Umbra is using', description: 'Processor, memory and graphics use.', icon: ActivityIcon }
];

interface SettingBase {
	key: string;
	label: string;
	group: string;
	icon: Component;
	/** Shown in the picker. */
	description: string;
}

export type SettingDef = SettingBase &
	(
		| { type: 'toggle'; get: (s: Settings) => boolean; set: (s: Settings, v: boolean) => void }
		| {
				type: 'choice';
				options: { value: string; label: string }[];
				get: (s: Settings) => string;
				set: (s: Settings, v: string) => void;
		  }
		| {
				type: 'number';
				min: number;
				max: number;
				step: number;
				unit: string;
				get: (s: Settings) => number;
				set: (s: Settings, v: number) => void;
		  }
		| {
				type: 'slider';
				min: number;
				max: number;
				step: number;
				format: (v: number) => string;
				get: (s: Settings) => number;
				set: (s: Settings, v: number) => void;
		  }
	);

const secs = (values: number[]) => values.map((v) => ({ value: String(v), label: v >= 60 ? `${v / 60} min` : `${v} s` }));

export const SETTINGS: SettingDef[] = [
	// Clips
	{
		key: 'clip.lengthSecs', label: 'Clip length', group: 'Clips', icon: FilmIcon, type: 'choice',
		description: 'How much a clip keeps from before the hotkey.',
		options: secs([15, 30, 60, 90, 120, 300]),
		get: (s) => String(s.clip.lengthSecs), set: (s, v) => (s.clip.lengthSecs = Number(v))
	},
	{
		key: 'clip.postSecs', label: 'After the hotkey', group: 'Clips', icon: FilmIcon, type: 'choice',
		description: 'Seconds still recorded after the hotkey before the clip is saved.',
		options: [{ value: '0', label: 'None' }, ...secs([3, 5, 10])],
		get: (s) => String(s.clip.postSecs), set: (s, v) => (s.clip.postSecs = Number(v))
	},
	{
		key: 'clip.saveSound', label: 'Clip sound', group: 'Clips', icon: BellIcon, type: 'toggle',
		description: 'A short sound when a clip is saved.',
		get: (s) => s.clip.saveSound, set: (s, v) => (s.clip.saveSound = v)
	},
	{
		key: 'clip.saveSoundVolume', label: 'Clip sound volume', group: 'Clips', icon: Volume2Icon, type: 'slider',
		description: 'How loud the clip sound is.', min: 0, max: 1, step: 0.05, format: (v) => `${Math.round(v * 100)}%`,
		get: (s) => s.clip.saveSoundVolume, set: (s, v) => (s.clip.saveSoundVolume = v)
	},
	{
		key: 'clip.bufferOnDisk', label: 'Buffer on disk', group: 'Clips', icon: HardDriveIcon, type: 'toggle',
		description: 'Keep the replay buffer in files instead of memory.',
		get: (s) => s.clip.bufferOnDisk, set: (s, v) => (s.clip.bufferOnDisk = v)
	},
	{
		key: 'screenshot.format', label: 'Screenshot format', group: 'Clips', icon: CameraIcon, type: 'choice',
		description: 'PNG keeps every pixel, JPG is smaller.',
		options: [{ value: 'png', label: 'PNG' }, { value: 'jpg', label: 'JPG' }],
		get: (s) => s.screenshot.format, set: (s, v) => (s.screenshot.format = v as 'png' | 'jpg')
	},
	{
		key: 'screenshot.sound', label: 'Screenshot sound', group: 'Clips', icon: CameraIcon, type: 'toggle',
		description: 'A short sound when a screenshot is taken.',
		get: (s) => s.screenshot.sound, set: (s, v) => (s.screenshot.sound = v)
	},
	{
		key: 'overlay.enabled', label: 'Game messages', group: 'Clips', icon: LayersIcon, type: 'toggle',
		description: 'A short note over the game when a clip is saved or a recording starts.',
		get: (s) => s.overlay.enabled, set: (s, v) => (s.overlay.enabled = v)
	},
	{
		key: 'overlay.status', label: 'Status badge', group: 'Clips', icon: LayersIcon, type: 'choice',
		description: 'The badge that stays over the game.',
		options: [{ value: 'off', label: 'Off' }, { value: 'always', label: 'Always' }, { value: 'games', label: 'In games' }],
		get: (s) => s.overlay.status, set: (s, v) => (s.overlay.status = v as 'off' | 'always' | 'games')
	},

	// Recording
	{
		key: 'recording.mode', label: 'Replay buffer runs', group: 'Recording', icon: RadioIcon, type: 'choice',
		description: 'All the time, only while a game runs, or when you turn it on.',
		options: [{ value: 'always', label: 'Always' }, { value: 'games', label: 'In games' }, { value: 'manual', label: 'By hand' }],
		get: (s) => s.recording.mode, set: (s, v) => (s.recording.mode = v as Settings['recording']['mode'])
	},
	{
		key: 'recording.fullSessions', label: 'Game sessions', group: 'Recording', icon: Gamepad2Icon, type: 'toggle',
		description: 'Record every game session from start to end.',
		get: (s) => s.recording.fullSessions, set: (s, v) => (s.recording.fullSessions = v)
	},
	{
		key: 'recording.sessionSource', label: 'Sessions record', group: 'Recording', icon: AppWindowIcon, type: 'choice',
		description: 'What a game session records.',
		options: [{ value: 'capture', label: 'The capture' }, { value: 'gameWindow', label: 'Game window' }],
		get: (s) => s.recording.sessionSource, set: (s, v) => (s.recording.sessionSource = v as Settings['recording']['sessionSource'])
	},
	{
		key: 'recording.launchAtStartup', label: 'Start with Windows', group: 'Recording', icon: PowerIcon, type: 'toggle',
		description: 'Umbra starts when you sign in.',
		get: (s) => s.recording.launchAtStartup, set: (s, v) => (s.recording.launchAtStartup = v)
	},
	{
		key: 'recording.startMinimized', label: 'Start in the tray', group: 'Recording', icon: AppWindowMacIcon, type: 'toggle',
		description: 'Started with Windows, the window stays closed.',
		get: (s) => s.recording.startMinimized, set: (s, v) => (s.recording.startMinimized = v)
	},

	// Video
	{
		key: 'video.fps', label: 'Frame rate', group: 'Video', icon: MonitorIcon, type: 'choice',
		description: 'Frames per second recorded.',
		options: [30, 60, 120, 144, 165, 240].map((v) => ({ value: String(v), label: `${v}` })),
		get: (s) => String(s.video.fps), set: (s, v) => (s.video.fps = Number(v))
	},
	{
		key: 'video.codec', label: 'Codec', group: 'Video', icon: CpuIcon, type: 'choice',
		description: 'H.264 plays everywhere; HEVC and AV1 are smaller for the same quality.',
		options: [{ value: 'h264', label: 'H.264' }, { value: 'hevc', label: 'HEVC' }, { value: 'av1', label: 'AV1' }],
		get: (s) => s.video.codec, set: (s, v) => (s.video.codec = v as Settings['video']['codec'])
	},
	{
		key: 'video.bitrateKbps', label: 'Bitrate', group: 'Video', icon: CpuIcon, type: 'number',
		description: 'Video bitrate, for constant and variable bitrate.', min: 1000, max: 300_000, step: 1000, unit: 'kbit/s',
		get: (s) => s.video.bitrateKbps, set: (s, v) => (s.video.bitrateKbps = v)
	},
	{
		key: 'video.captureCursor', label: 'Cursor', group: 'Video', icon: MousePointerIcon, type: 'toggle',
		description: 'The mouse pointer is in the picture.',
		get: (s) => s.video.captureCursor, set: (s, v) => (s.video.captureCursor = v)
	},
	{
		key: 'video.hdr', label: 'HDR screens', group: 'Video', icon: MonitorIcon, type: 'choice',
		description: 'Convert to SDR, or keep HDR10.',
		options: [{ value: 'sdr', label: 'As SDR' }, { value: 'hdr10', label: 'HDR10' }],
		get: (s) => s.video.hdr, set: (s, v) => (s.video.hdr = v as 'sdr' | 'hdr10')
	},

	// Audio
	{
		key: 'audio.mic.enabled', label: 'Microphone', group: 'Audio', icon: MicIcon, type: 'toggle',
		description: 'Record your microphone.',
		get: (s) => s.audio.mic.enabled, set: (s, v) => (s.audio.mic.enabled = v)
	},
	{
		key: 'camera.enabled', label: 'Camera', group: 'Camera', icon: VideoIcon, type: 'toggle',
		description: 'Record your webcam next to the screen.',
		get: (s) => s.camera.enabled, set: (s, v) => (s.camera.enabled = v)
	},
	{
		key: 'audio.mic.pushToTalk', label: 'Push to talk', group: 'Audio', icon: MicIcon, type: 'toggle',
		description: 'The microphone is recorded only while the key is held.',
		get: (s) => s.audio.mic.pushToTalk, set: (s, v) => (s.audio.mic.pushToTalk = v)
	},
	{
		key: 'audio.game', label: 'Game sound', group: 'Audio', icon: Gamepad2Icon, type: 'toggle',
		description: "The game's own track.",
		get: (s) => s.audio.game, set: (s, v) => (s.audio.game = v)
	},
	{
		key: 'audio.others', label: 'Other apps', group: 'Audio', icon: Volume2Icon, type: 'toggle',
		description: 'Everything else you hear, on a track of its own.',
		get: (s) => s.audio.others, set: (s, v) => (s.audio.others = v)
	},
	{
		key: 'audio.bitrateKbps', label: 'Audio quality', group: 'Audio', icon: Volume2Icon, type: 'choice',
		description: 'Bitrate of each audio track.',
		options: [128, 160, 192, 256, 320].map((v) => ({ value: String(v), label: `${v}` })),
		get: (s) => String(s.audio.bitrateKbps), set: (s, v) => (s.audio.bitrateKbps = Number(v))
	},

	// Storage
	{
		key: 'storage.limitGb', label: 'Folder limit', group: 'Storage', icon: DatabaseIcon, type: 'number',
		description: 'Keep the clip folder under this size by removing the oldest clips; 0 is off.', min: 0, max: 100_000, step: 10, unit: 'GB',
		get: (s) => s.storage.limitGb, set: (s, v) => (s.storage.limitGb = v)
	},
	{
		key: 'storage.maxAgeDays', label: 'Keep clips for', group: 'Storage', icon: DatabaseIcon, type: 'number',
		description: 'Remove clips older than this; 0 is off.', min: 0, max: 3650, step: 1, unit: 'days',
		get: (s) => s.storage.maxAgeDays, set: (s, v) => (s.storage.maxAgeDays = v)
	},
	{
		key: 'storage.minFreeGb', label: 'Keep free', group: 'Storage', icon: HardDriveIcon, type: 'number',
		description: 'Stop recording when the drive has less free than this.', min: 0, max: 1000, step: 1, unit: 'GB',
		get: (s) => s.storage.minFreeGb, set: (s, v) => (s.storage.minFreeGb = v)
	},
	{
		key: 'storage.skipRecycleBin', label: 'Skip Recycle Bin', group: 'Storage', icon: Trash2Icon, type: 'toggle',
		description: 'Deleted clips are gone for good instead of going to the Recycle Bin.',
		get: (s) => s.storage.skipRecycleBin, set: (s, v) => (s.storage.skipRecycleBin = v)
	},

	// Player
	{
		key: 'player.seekSecs', label: 'Arrow keys jump', group: 'Player', icon: FilmIcon, type: 'choice',
		description: 'How far the arrow keys jump in the player.',
		options: [{ value: '2', label: '2 s' }, { value: '5', label: '5 s' }, { value: '10', label: '10 s' }],
		get: (s) => String(s.player.seekSecs), set: (s, v) => (s.player.seekSecs = Number(v))
	}
];

export const SETTING_GROUPS = ['Clips', 'Recording', 'Video', 'Audio', 'Camera', 'Storage', 'Player'];

export type WidgetDef = BlockWidget | (SettingDef & { kind: 'setting'; id: string });

/** The widget an id names, or nothing for an id this version does not know. */
export function widgetFor(id: string): WidgetDef | undefined {
	if (id.startsWith('setting:')) {
		const def = SETTINGS.find((s) => s.key === id.slice('setting:'.length));
		return def ? { ...def, kind: 'setting', id } : undefined;
	}
	return BLOCKS.find((b) => b.id === id);
}

/** Toggles take half the width, two side by side; everything else the full width. */
export function isHalf(id: string): boolean {
	const def = widgetFor(id);
	return def?.kind === 'setting' && def.type === 'toggle';
}
