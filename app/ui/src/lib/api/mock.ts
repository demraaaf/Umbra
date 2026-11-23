// In-memory mock of the whole backend, so the UI runs in any browser (`npm run dev`).
// It behaves like the real engine at the level the UI can observe: the buffer fills while
// "recording", usage ticks, saving a clip or screenshot takes a moment and emits events, exports
// report progress, albums live in memory, and thumbnails for new clips arrive a moment later.

import {
	AUDIO_APPS,
	DEFAULT_SETTINGS,
	GAMES,
	MICROPHONES,
	CAMERAS,
	MONITORS,
	SYSTEM_INFO,
	WINDOWS,
	buildClip,
	buildImage,
	makePreview,
	makeThumbnail,
	mockVideoUrl,
	pickFormat,
	rng,
	seedAlbums,
	seedClips,
	seedScreenshots
} from './mock-data';
import { mockInputTrack } from '$lib/input/overlay';
import type {
	Album,
	Api,
	AutoClipLogEntry,
	BenchmarkProgress,
	Cleanup,
	LoadProgress,
	LowDisk,
	ImportDone,
	ImportProgress,
	FileDrop,
	ShrinkResult,
	BurnResult,
	BenchmarkTrial,
	RecordingFinishing,
	RecordingKind,
	RecordingSaved,
	CaptureSource,
	CaptureTarget,
	Clip,
	ClipAudioTrack,
	ExportProgress,
	ExportRequest,
	MixRole,
	Settings,
	Status,
	UsageEstimate,
	VideoCodec
} from './types';

/** Tracks as the recorder lays them out: a combined `Mix` first when there is more than one. */
function recorderTracks(parts: [string, MixRole][]): ClipAudioTrack[] {
	if (parts.length === 1) return [{ index: 0, title: parts[0][0], role: 'separate' }];
	return [
		{ index: 0, title: 'Mix', role: 'mix' },
		...parts.map(([title, role], i) => ({ index: i + 1, title, role }))
	];
}


const SETTINGS_KEY = 'umbra.mock.settings.v2';
const SAMPLE_FOLDERS = ['D:\\Clips', 'E:\\Recordings\\Umbra', 'C:\\Users\\you\\Videos\\Umbra'];
const MB = 1_000_000;

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));
const latency = () => sleep(40 + Math.random() * 90);

/** The mock's logo: a browser cannot read the chosen file, so a drawn one stands in. */
const MOCK_LOGO =
	'data:image/svg+xml,' +
	encodeURIComponent(
		'<svg xmlns="http://www.w3.org/2000/svg" width="240" height="80"><rect width="240" height="80" rx="10" fill="#111" fill-opacity=".6"/><text x="120" y="52" font-family="sans-serif" font-size="34" font-weight="700" fill="#fff" text-anchor="middle">LOGO</text></svg>'
	);
const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value)) as T;
const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/** Stored settings merged over the defaults one group deep, so new fields get their defaults. */
function loadSettings(): Settings {
	const base = clone(DEFAULT_SETTINGS);
	try {
		const raw = localStorage.getItem(SETTINGS_KEY);
		if (!raw) return base;
		const stored = JSON.parse(raw) as Partial<Settings>;
		for (const key of Object.keys(base) as (keyof Settings)[]) {
			if (stored[key] && typeof stored[key] === 'object') {
				(base as unknown as Record<string, unknown>)[key] = { ...base[key], ...stored[key] };
			}
		}
		base.audio.mic = { ...DEFAULT_SETTINGS.audio.mic, ...(stored.audio?.mic ?? {}) };
		if (stored.capture) base.capture = stored.capture;
	} catch {
		/* storage can be blocked; fall back to defaults */
	}
	return base;
}

function persistSettings(settings: Settings) {
	try {
		localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
	} catch {
		/* ignore */
	}
}

/** Mirrors what the engine would enforce: ranges, whole numbers, both-or-neither size. */
function validate(input: Settings): Settings {
	const s = clone(input);
	const whole = (v: number, lo: number, hi: number) => clamp(Math.round(Number.isFinite(v) ? v : lo), lo, hi);
	s.video.fps = whole(s.video.fps, 1, 360);
	s.video.qp = whole(s.video.qp, 0, 51);
	s.video.preset = whole(s.video.preset, 1, 7);
	s.video.bitrateKbps = Math.max(1, Math.round(s.video.bitrateKbps));
	s.video.maxBitrateKbps = Math.max(s.video.bitrateKbps, Math.round(s.video.maxBitrateKbps));
	if (s.video.width == null || s.video.height == null) {
		s.video.width = null;
		s.video.height = null;
	} else {
		s.video.width = whole(s.video.width, 16, 16384);
		s.video.height = whole(s.video.height, 16, 16384);
	}
	s.audio.bitrateKbps = Math.max(1, Math.round(s.audio.bitrateKbps));
	s.clip.lengthSecs = whole(s.clip.lengthSecs, 1, 3600);
	s.clip.saveSoundVolume = clamp(s.clip.saveSoundVolume, 0, 1);
	return s;
}

function sourceSize(settings: Settings): { name: string; width: number; height: number } {
	const c = settings.capture;
	if (c.kind === 'window') return { name: c.exe, width: 1920, height: 1080 };
	const m = MONITORS.find((x) => x.device === c.device) ?? MONITORS.find((x) => x.primary)!;
	return { name: m.device.replace('\\\\.\\', ''), width: m.width, height: m.height };
}

function trackCount(settings: Settings): number {
	const a = settings.audio;
	return (a.mode === 'system' ? 1 : (a.game ? 1 : 0) + a.apps.length + (a.others ? 1 : 0)) + (a.mic.enabled ? 1 : 0);
}

/** Plausible cost model: bitrate for memory and size, pixel rate against an NVENC budget for load. */
export function estimate(settings: Settings): UsageEstimate {
	const v = settings.video;
	const src = sourceSize(settings);
	const width = v.width ?? src.width;
	const height = v.height ?? src.height;
	const pixelRate = width * height * v.fps;
	const tracks = trackCount(settings);
	const audioKbps = tracks * settings.audio.bitrateKbps;

	let avgKbps: number;
	let peakKbps: number;
	if (v.rateMode === 'cbr') avgKbps = peakKbps = v.bitrateKbps;
	else if (v.rateMode === 'vbr') {
		avgKbps = v.bitrateKbps;
		peakKbps = v.maxBitrateKbps;
	} else {
		// Constant quality: bits per pixel roughly halve every 6 QP steps.
		const bitsPerPixel = 0.09 * Math.pow(2, (23 - v.qp) / 6);
		avgKbps = (pixelRate * bitsPerPixel) / 1000 / 2.5;
		peakKbps = avgKbps * 1.6;
	}

	const len = settings.clip.lengthSecs;
	const bufferBytes = ((peakKbps + audioKbps) * 1000 / 8) * len;
	const clipBytes = ((avgKbps + audioKbps) * 1000 / 8) * len;

	const codecCost = { h264: 1, hevc: 1.15, av1: 1.35 }[v.codec];
	const presetCost = 0.7 + (v.preset - 1) * 0.15;
	const encoderLoad = (pixelRate * codecCost * presetCost) / (2560 * 1440 * 420);
	const cpuPercent = clamp(1.2 + tracks * 0.35 + (pixelRate / (2560 * 1440 * 360)) * 2.5 + (v.captureCursor ? 0.2 : 0), 0, 100);

	const notes: string[] = [];
	let level: UsageEstimate['level'] = 'light';
	if (encoderLoad > 1) {
		level = 'too-heavy';
		notes.push('The video encoder cannot keep up with this resolution and frame rate. Lower the frame rate, the resolution or the encoder preset.');
	} else if (encoderLoad > 0.75 || bufferBytes > 3000 * MB) {
		level = 'heavy';
	} else if (encoderLoad > 0.45 || bufferBytes > 1200 * MB) {
		level = 'moderate';
	}
	if (encoderLoad > 0.75 && encoderLoad <= 1) {
		notes.push('The encoder is close to its limit, so games that also load the GPU may drop frames.');
	}
	if (bufferBytes > 1200 * MB) {
		notes.push(`The replay buffer keeps about ${Math.round(bufferBytes / MB).toLocaleString('en-US')} MB in memory while recording.`);
	}
	if (v.fps >= 240) notes.push('High frame rates multiply the encoder’s work and the size of every clip.');
	if (v.rateMode === 'cqp' || v.rateMode === 'cq') notes.push('With constant quality, sizes depend on what is on screen; these numbers are a typical case.');
	if (level === 'light' && notes.length === 0) notes.push('Comfortable for this PC.');
	if (level === 'moderate' && notes.length === 0) notes.push('Fine for most PCs, though a demanding game will compete with Umbra for the graphics card.');

	return {
		bufferBytes: Math.round(bufferBytes),
		clipBytes: Math.round(clipBytes),
		encoderLoad: Math.round(encoderLoad * 100) / 100,
		cpuPercent: Math.round(cpuPercent * 10) / 10,
		level,
		notes,
		breakdown: [
			{
				resource: 'encoder',
				impact: encoderLoad > 0.5 ? 'high' : encoderLoad > 0.2 ? 'medium' : 'low',
				title: `${width}×${height} at ${v.fps} fps`,
				why: `The encoder compresses every pixel of every frame, about ${Math.round(encoderLoad * 100)}% of what it can do with these settings.`,
				change: 'Lower Video › Frame rate, or record a smaller Video › Resolution.',
				catch: 'Fewer frames look less smooth on a high-refresh screen; a smaller size is less sharp.'
			},
			...(v.skipStill
				? []
				: [
						{
							resource: 'encoder' as const,
							impact: 'medium' as const,
							title: 'Still frames are encoded again',
							why: 'To keep a steady frame rate, the last picture is encoded again whenever the screen has not changed.',
							change: 'Turn on Video › Skip still frames (with advanced settings on).',
							catch: 'The frame rate varies; Premiere and DaVinci Resolve may warn or let the audio drift.'
						}
					]),
			{
				resource: 'memory',
				impact: bufferBytes > 1200 * MB ? 'medium' : 'low',
				title: `Replay buffer: ${Math.round(bufferBytes / MB).toLocaleString('en-US')} MB of RAM`,
				why: `The last ${len} s are kept in memory. Its size is the bitrate times the clip length.`,
				change: 'Lower the video bitrate, or shorten Clips › Length.',
				catch: 'Too little bitrate makes fast motion blocky.'
			}
		]
	};
}

function targetKey(target: CaptureTarget): string {
	return target.kind === 'monitor' ? `monitor:${target.device}` : `window:${target.exe}:${target.title}`;
}

export function createMockApi(): Api {
	let settings = loadSettings();
	let clips = [...seedClips(), ...seedScreenshots()];
	let albums: Album[] = seedAlbums(clips);
	const thumbnailReadyAt = new Map<string, number>();

	const statusHandlers = new Set<(s: Status) => void>();
	const savedHandlers = new Set<(c: Clip) => void>();
	const progressHandlers = new Set<(p: ExportProgress) => void>();
	const clipsChangedHandlers = new Set<() => void>();
	const albumsChangedHandlers = new Set<() => void>();
	const benchmarkHandlers = new Set<(p: BenchmarkProgress) => void>();
	const loadHandlers = new Set<(p: LoadProgress) => void>();
	const importProgressHandlers = new Set<(p: ImportProgress) => void>();
	const importDoneHandlers = new Set<(d: ImportDone) => void>();
	const recordingHandlers = new Set<(r: RecordingSaved) => void>();
	/** The mock's recording to a file: when it started and what kind it is. */
	// While paused, `pausedAt` holds when; `pausedFor` adds up the time spent paused.
	let fileRecording: { kind: RecordingKind; started: number; pausedAt: number | null; pausedFor: number } | null = null;
	const recordedSecs = (f: NonNullable<typeof fileRecording>) =>
		((f.pausedAt ?? Date.now()) - f.started - f.pausedFor) / 1000;
	let benchmarkCancelled = false;

	// A few library clips start without a thumbnail, as the real backend reports them while it
	// generates previews in the background. The UI must show a plain tile and then update.
	const now = Date.now();
	[2, 5, 11].forEach((i) => clips[i] && thumbnailReadyAt.set(clips[i].path, now + 2400));

	const timers = new Map<Set<() => void>, ReturnType<typeof setTimeout>>();
	/** Folds bursts into one notification, like the backend's change events. */
	function notify(set: Set<() => void>, delay = 0) {
		if (timers.has(set)) return;
		timers.set(
			set,
			setTimeout(() => {
				timers.delete(set);
				set.forEach((h) => h());
			}, delay)
		);
	}
	const clipsChanged = (delay = 0) => notify(clipsChangedHandlers, delay);
	const albumsChanged = () => notify(albumsChangedHandlers);

	// ---------- Live status ----------
	let recording = settings.recording.mode !== 'manual';
	let bufferSecs = recording ? Math.min(settings.clip.lengthSecs, 34) : 0;
	let droppedFrames = 0;
	let activeGame: string | null = recording ? 'Valorant' : null;
	let pttHeld = false;
	let cpu = 3.5;
	let gpu = 6;

	function bytesPerSecond(): number {
		const v = settings.video;
		const videoKbps = v.rateMode === 'cqp' ? 60000 : v.rateMode === 'cq' ? v.maxBitrateKbps : v.bitrateKbps;
		return ((videoKbps + trackCount(settings) * settings.audio.bitrateKbps) * 1000) / 8;
	}

	function snapshot(): Status {
		const src = sourceSize(settings);
		const v = settings.video;
		const mic = settings.audio.mic;
		const est = estimate(settings);
		const buffer = Math.round(bufferSecs * bytesPerSecond());
		return {
			state: recording ? 'recording' : 'idle',
			error: null,
			target: recording ? `${src.name}, ${src.width}x${src.height}` : null,
			game: recording ? activeGame : null,
			gameReason: recording && activeGame ? 'installed through Steam' : null,
			encoder: recording ? `${v.codec}_nvenc` : null,
			width: v.width ?? src.width,
			height: v.height ?? src.height,
			fps: v.fps,
			bufferSecs: Math.floor(bufferSecs),
			bufferBytes: buffer,
			bufferOnDisk: settings.clip.bufferOnDisk,
			micLive: recording && mic.enabled && (!mic.pushToTalk || pttHeld),
			pushToTalk: mic.pushToTalk,
			droppedFrames,
			usage: {
				cpuPercent: Math.round((recording ? cpu : 0.4) * 10) / 10,
				memoryBytes: 168 * MB + (recording ? buffer : 0),
				gpuPercent: recording ? Math.round(gpu) : 0,
				encoderPercent: recording ? Math.round(clamp(est.encoderLoad * 100 + (Math.random() - 0.5) * 6, 0, 100)) : 0
			},
			sessionStopped: false,
			audioProblems: [],
			cameraOn: settings.camera.enabled,
			cameraLive: recording && settings.camera.enabled && settings.camera.deviceId !== null,
			cameraProblem: null,
			input: {
				on: settings.input.enabled,
				app: settings.input.enabled ? 'cs2.exe' : null,
				rule: settings.input.enabled ? 'Games' : null,
				guard: null
			},
			file: fileRecording
				? {
						kind: fileRecording.kind,
						secs: recordedSecs(fileRecording),
						bytes: Math.round(recordedSecs(fileRecording) * bytesPerSecond()),
						paused: fileRecording.pausedAt !== null,
						game: activeGame ?? 'Desktop',
						path: 'D:\\Clips\\Desktop\\Recording.mp4',
						source: settings.capture
					}
				: null
		};
	}

	function emitStatus() {
		const s = snapshot();
		statusHandlers.forEach((h) => h(s));
	}

	if (typeof window !== 'undefined') {
		setInterval(() => {
			if (recording) {
				bufferSecs = Math.min(settings.clip.lengthSecs, bufferSecs + 1);
				const heavy = settings.video.fps >= 240;
				if (Math.random() < (heavy ? 0.12 : 0.008)) droppedFrames += 1 + Math.floor(Math.random() * 3);
				const target = estimate(settings).cpuPercent;
				cpu = clamp(cpu + (target - cpu) * 0.3 + (Math.random() - 0.5) * 0.8, 0.5, 100);
				gpu = clamp(gpu + (Math.random() - 0.5) * 2, 3, 14);
			}
			// Simulate someone pressing and releasing the push-to-talk key now and then.
			if (settings.audio.mic.pushToTalk && Math.random() < 0.3) pttHeld = !pttHeld;
			emitStatus();
		}, 1000);
	}

	/** New clips have no thumbnail for a moment, like the real pipeline. */
	function withThumbnailDelay(clip: Clip): Clip {
		const readyAt = thumbnailReadyAt.get(clip.path);
		if (readyAt && Date.now() < readyAt) return { ...clip, thumbnailUrl: null };
		return clip;
	}

	/** Album membership is stored on albums; clips report it back as ids. */
	function withAlbums(clip: Clip): Clip {
		return { ...clip, albums: albums.filter((a) => a.clipPaths.includes(clip.path)).map((a) => a.id) };
	}

	function present(clip: Clip): Clip {
		return clone(withAlbums(withThumbnailDelay(clip)));
	}

	function dropFromAlbums(paths: string[]) {
		let changed = false;
		albums = albums.map((a) => {
			const kept = a.clipPaths.filter((p) => !paths.includes(p));
			if (kept.length !== a.clipPaths.length) changed = true;
			return { ...a, clipPaths: kept };
		});
		if (changed) albumsChanged();
	}

	function findAlbum(id: string): Album {
		const album = albums.find((a) => a.id === id);
		if (!album) throw new Error('That album no longer exists.');
		return album;
	}

	function updateAlbum(id: string, fn: (a: Album) => Album): Album {
		findAlbum(id);
		albums = albums.map((a) => (a.id === id ? fn(a) : a));
		albumsChanged();
		return clone(findAlbum(id));
	}

	function unsubscribe<T>(set: Set<T>, handler: T) {
		return () => {
			set.delete(handler);
		};
	}

	function subscribe<T>(set: Set<T>) {
		return async (handler: T) => {
			set.add(handler);
			return unsubscribe(set, handler);
		};
	}

	return {
		async getSettings() {
			await latency();
			return clone(settings);
		},

		async setSettings(next) {
			await latency();
			settings = validate(next);
			persistSettings(settings);
			if (settings.clip.lengthSecs < bufferSecs) bufferSecs = settings.clip.lengthSecs;
			emitStatus();
			return clone(settings);
		},

		async getStatus() {
			await latency();
			return snapshot();
		},

		async getSystemInfo() {
			await latency();
			return clone(SYSTEM_INFO);
		},

		async startRecording() {
			await latency();
			recording = true;
			bufferSecs = 0;
			droppedFrames = 0;
			activeGame = 'Valorant';
			emitStatus();
		},

		async stopRecording() {
			await latency();
			recording = false;
			bufferSecs = 0;
			activeGame = null;
			emitStatus();
		},

		async saveClip() {
			if (!recording) throw new Error('Recording is not running, so there is nothing to save.');
			await sleep(320);
			const secs = Math.min(Math.floor(bufferSecs), settings.clip.lengthSecs);
			if (secs < 1) throw new Error('The buffer is empty. Try again in a moment.');
			const r = rng(Date.now() % 100000);
			const format = pickFormat(r);
			const src = sourceSize(settings);
			const clip = buildClip({
				seed: Math.floor(r() * 1000) + 100,
				game: activeGame,
				createdAt: new Date(),
				durationSecs: secs,
				format: {
					...format,
					width: settings.video.width ?? src.width,
					height: settings.video.height ?? src.height,
					fps: settings.video.fps,
					codec: settings.video.codec as VideoCodec
				},
				tracks: recorderTracks([
					['Game', 'inMix'],
					...settings.audio.apps.map(
						(exe): [string, MixRole] => [
							AUDIO_APPS.find((a) => a.exe === exe)?.name ?? exe.replace(/\.exe$/i, ''),
							'inMix'
						]
					),
					['Other apps', 'inMix'],
					...(settings.audio.mic.enabled ? [['Microphone', 'separate'] as [string, MixRole]] : [])
				]),
				folder: settings.storage.folder
			});
			thumbnailReadyAt.set(clip.path, Date.now() + 1800);
			clips = [clip, ...clips];
			const delivered = present(clip);
			savedHandlers.forEach((h) => h(clone(delivered)));
			clipsChanged(600);
			return delivered;
		},

		async takeScreenshot() {
			await sleep(180);
			const src = sourceSize(settings);
			const shot = buildImage({
				seed: Math.floor(Math.random() * 1000) + 500,
				game: activeGame,
				createdAt: new Date(),
				width: src.width,
				height: src.height,
				format: settings.screenshot.format,
				folder: settings.storage.folder
			});
			clips = [shot, ...clips];
			const delivered = present(shot);
			savedHandlers.forEach((h) => h(clone(delivered)));
			clipsChanged();
			return delivered;
		},

		async listMonitors() {
			await latency();
			return clone(MONITORS);
		},
		async listRunningGames() {
			return [{ exe: 'cs2.exe', name: 'Counter-Strike 2' }];
		},
		async listWindows() {
			await latency();
			return clone(WINDOWS);
		},
		async listAudioApps() {
			await latency();
			// "Currently playing sound": a stable subset, so the suggestion chips are predictable.
			return clone(AUDIO_APPS.filter((a) => ['Discord.exe', 'chrome.exe', 'Spotify.exe'].includes(a.exe)));
		},
		async listMicrophones() {
			await latency();
			return clone(MICROPHONES);
		},
		async listCameras() {
			await latency();
			return clone(CAMERAS);
		},

		async listCaptureSources() {
			await latency();
			const gameExes = new Set([...GAMES.map((g) => g.exe.toLowerCase()), ...settings.recording.games.map((g) => g.toLowerCase())]);
			const monitors: CaptureSource[] = MONITORS.map((m, i) => ({
				target: { kind: 'monitor', device: m.device },
				label: `Display ${i + 1}`,
				detail: `${m.width}\u00d7${m.height} \u00b7 ${m.refreshHz} Hz`,
				isGame: false
			}));
			const windows: CaptureSource[] = WINDOWS.map((w) => ({
				target: { kind: 'window', exe: w.exe, title: w.title },
				label: w.title,
				detail: w.exe,
				isGame: gameExes.has(w.exe.toLowerCase())
			}));
			return clone([...monitors, ...windows]);
		},

		async getPreview(target, maxWidth) {
			await sleep(60 + Math.random() * 120);
			void maxWidth;
			// A minimised window has nothing to show.
			if (target.kind === 'window' && target.exe === 'explorer.exe') return null;
			return makePreview(targetKey(target), Math.floor(Date.now() / 5000));
		},

		async estimateUsage(proposed) {
			await sleep(30 + Math.random() * 60);
			return estimate(validate(proposed));
		},

		async runBenchmark(answers) {
			benchmarkCancelled = false;
			const screen = MONITORS.find((m) => m.primary)!;
			const fps = answers.fps ?? (answers.game === 'shooter' ? { least: 60, some: 120, most: 144 }[answers.cost] : 60);
			const [width, height] = answers.size ?? [screen.nativeWidth, screen.nativeHeight];
			const codec: VideoCodec = answers.share ? 'h264' : answers.cost === 'most' ? 'av1' : 'hevc';
			const need = fps * { least: 3, some: 2, most: 1.4 }[answers.cost];
			const trials: BenchmarkTrial[] = [];
			const emit = (p: BenchmarkProgress) => benchmarkHandlers.forEach((h) => h(p));
			emit({ label: 'Checking which codecs this graphics card can encode', trial: null });
			await sleep(500);
			for (const preset of [7, 6, 5, 4, 3, 1]) {
				if (benchmarkCancelled) throw new Error('cancelled');
				emit({ label: `Testing ${codec.toUpperCase()} at ${width}×${height}, ${fps} fps, preset ${preset}`, trial: null });
				await sleep(900);
				const measured = (2.6e9 / (width * height)) * (preset >= 6 ? 0.38 : 1) * (8 - preset) ** 0.15;
				const trial: BenchmarkTrial = { codec, width, height, fps, preset, measuredFps: measured, load: fps / measured, passed: measured >= need, error: null };
				trials.push(trial);
				emit({ label: '', trial });
				if (trial.passed) break;
			}
			const pick = trials.find((t) => t.passed) ?? trials[trials.length - 1];
			const kbps = Math.round((25 * (width * height / 2073600) ** 0.8 * (fps / 60) ** 0.6 * { least: 0.8, some: 1, most: 1.4 }[answers.cost])) * 1000;
			return clone({
				video: {
					...settings.video,
					codec,
					fps,
					preset: pick.preset,
					width: answers.size ? width : null,
					height: answers.size ? height : null,
					rateMode: 'vbr' as const,
					bitrateKbps: kbps,
					maxBitrateKbps: Math.round(kbps * 1.5),
					spatialAq: answers.cost !== 'least',
					multipass: answers.cost === 'most' ? ('quarter' as const) : ('off' as const),
					scaling: answers.scaling
				},
				trials,
				notes: [`The encoder managed ${Math.round(pick.measuredFps ?? 0)} fps at these settings, so recording ${fps} fps uses about ${Math.round((pick.load ?? 0) * 100)}% of it.`],
				compromise: !pick.passed
			});
		},

		async getStorageUsage() {
			return { bytes: clips.reduce((sum, c) => sum + c.sizeBytes, 0), files: clips.length, freeBytes: 412 * 2 ** 30 };
		},

		async cleanUpStorage() {
			await latency();
			return { files: 0, bytes: 0 };
		},

		async getDefaultSettings() {
			return clone(DEFAULT_SETTINGS);
		},

		// The browser preview shows the first-start screen with `?welcome` in the address.
		async isFirstRun() {
			return typeof location !== 'undefined' && new URLSearchParams(location.search).has('welcome');
		},

		async finishOnboarding() {
			await latency();
		},

		async startFileRecording(kind, _source) {
			await latency();
			fileRecording ??= { kind, started: Date.now(), pausedAt: null, pausedFor: 0 };
			recording = true;
			emitStatus();
		},

		async pauseFileRecording(paused) {
			await latency();
			if (!fileRecording || paused === (fileRecording.pausedAt !== null)) return;
			if (paused) {
				fileRecording.pausedAt = Date.now();
			} else {
				fileRecording.pausedFor += Date.now() - (fileRecording.pausedAt ?? Date.now());
				fileRecording.pausedAt = null;
			}
			emitStatus();
		},

		async stopFileRecording() {
			await latency();
			if (!fileRecording) return;
			const kind = fileRecording.kind;
			fileRecording = null;
			emitStatus();
			recordingHandlers.forEach((h) => h({ clip: null, kind, game: activeGame ?? 'Desktop', error: 'The browser preview does not write files.' }));
		},

		async runLoadTest() {
			for (let i = 1; i <= 10; i++) {
				if (benchmarkCancelled) throw new Error('cancelled');
				await sleep(300);
				loadHandlers.forEach((h) => h({ label: i < 5 ? 'Measuring the game on its own' : 'Measuring the game while recording', fraction: i / 10 }));
			}
			return {
				alone: { fps: 198, lowFps: 171 },
				recording: { fps: 186, lowFps: 139 },
				fpsCost: 6.1,
				lowCost: 18.7,
				framesRecorded: 1150,
				framesDropped: 0,
				dropPercent: 0,
				gameMemory: 8 * 2 ** 30,
				bufferMemory: 0.4 * 2 ** 30,
				totalMemory: 32 * 2 ** 30,
				spec: { width: 2560, height: 1440, targetFps: 200, cpuThreads: 8 },
				suggestions: [
					{ text: 'Record 120 fps instead of 144.', video: { ...settings.video, fps: 120 }, clipSecs: null }
				]
			};
		},

		async cancelBenchmark() {
			benchmarkCancelled = true;
		},

		async videoSettingsCode(video) {
			// Not the real format: the browser mock only needs a code that reads back.
			const bytes = new TextEncoder().encode(JSON.stringify(video));
			const text = btoa(String.fromCharCode(...bytes)).replace(/=+$/, '').toUpperCase();
			return 'UMBRA-MOCK-' + text.slice(0, 30);
		},

		async readVideoSettingsCode(code) {
			await latency();
			if (!/^\s*umbra/i.test(code)) throw new Error("That isn't a settings code.");
			return clone({ ...settings.video, fps: 120, codec: 'av1' as const, preset: 6 });
		},

		async openSettingsFolder() {
			await latency();
			console.info('[mock] open settings folder:', SYSTEM_INFO.settingsFolder);
		},

		async listClips() {
			await latency();
			const list = [...clips].sort((a, b) => b.createdAt.localeCompare(a.createdAt)).map(present);
			// Thumbnails still pending: announce a change once they may be ready.
			if (list.some((c) => c.thumbnailUrl === null)) clipsChanged(600);
			return list;
		},

		async deleteClip(path) {
			await latency();
			if (!clips.some((c) => c.path === path)) throw new Error('That clip no longer exists.');
			clips = clips.filter((c) => c.path !== path);
			dropFromAlbums([path]);
			clipsChanged();
		},

		async deleteClips(paths) {
			await sleep(120);
			const set = new Set(paths);
			clips = clips.filter((c) => !set.has(c.path));
			dropFromAlbums(paths);
			clipsChanged();
		},

		async renameClip(path, newName) {
			await latency();
			const index = clips.findIndex((c) => c.path === path);
			if (index < 0) throw new Error('That clip no longer exists.');
			const extension = clips[index].fileName.match(/\.[^.]+$/)?.[0] ?? '.mp4';
			const name = newName.trim().replace(/\.(mp4|png|jpg)$/i, '');
			if (!name) throw new Error('The name cannot be empty.');
			if (/[<>:"/\\|?*]/.test(name)) throw new Error('A file name cannot contain < > : " / \\ | ? *');
			const fileName = `${name}${extension}`;
			const folder = path.slice(0, path.lastIndexOf('\\'));
			const newPath = `${folder}\\${fileName}`;
			if (newPath !== path && clips.some((c) => c.path === newPath)) {
				throw new Error('A clip with that name already exists.');
			}
			const old = clips[index];
			const renamed: Clip = {
				...old,
				fileName,
				path: newPath,
				videoUrl: old.kind === 'image' ? old.videoUrl : mockVideoUrl(newPath)
			};
			clips = clips.map((c, i) => (i === index ? renamed : c));
			albums = albums.map((a) => ({ ...a, clipPaths: a.clipPaths.map((p) => (p === path ? newPath : p)) }));
			albumsChanged();
			clipsChanged();
			return present(renamed);
		},

		async revealClip(path) {
			await latency();
			console.info('[mock] reveal in folder:', path);
		},

		async dragClipsOut() {

			return false;

		},

		async copyClipsToClipboard() {
			await latency();
			// A browser page cannot put files on the clipboard; the preview just accepts it.
		},

		async exportClip(request: ExportRequest) {
			const source = clips.find((c) => c.path === request.path);
			if (!source) throw new Error('That clip no longer exists.');
			if (request.endSecs <= request.startSecs) throw new Error('The end point must be after the start point.');
			const steps = 24;
			for (let i = 1; i <= steps; i++) {
				await sleep(110);
				const progress: ExportProgress = { path: request.path, fraction: i / steps };
				progressHandlers.forEach((h) => h(progress));
			}
			const cutSecs = request.cuts.reduce((sum, c) => sum + Math.max(0, c.endSecs - c.startSecs), 0);
			const durationSecs = Math.max(0.1, request.endSecs - request.startSecs - cutSecs);
			const ratio = durationSecs / source.durationSecs;
			const cropRatio = request.crop ? (request.crop.width * request.crop.height) / (source.width * source.height) : 1;
			const kept = request.audioTracks.map((t) => t.index);
			const tracks = source.audioTracks.filter((t) => kept.includes(t.index));
			const stem = source.fileName.replace(/\.mp4$/i, '');
			const fileName = request.overwrite ? source.fileName : `${request.name?.trim() || `${stem} (edit)`}.mp4`;
			const path = request.overwrite ? source.path : source.path.replace(source.fileName, fileName);
			const result: Clip = {
				...source,
				path,
				fileName,
				durationSecs,
				width: request.crop?.width ?? source.width,
				height: request.crop?.height ?? source.height,
				sizeBytes: Math.round(source.sizeBytes * ratio * (0.4 + 0.6 * cropRatio)),
				createdAt: request.overwrite ? source.createdAt : new Date().toISOString(),
				audioTracks: tracks.map((t, i) => ({ ...t, index: i })),
				videoUrl: mockVideoUrl(path),
				thumbnailUrl: source.thumbnailUrl ?? makeThumbnail(7, source.game)
			};
			const format = request.format ?? 'mp4';
			if (format !== 'mp4') {
				// Other formats are not clips in the library; the preview only reports the file.
				const other = path.replace(/\.mp4$/i, `.${format}`);
				return { clip: null, path: other, fileName: other.split(/[\\/]/).pop() ?? other, sizeBytes: result.sizeBytes };
			}
			clips = request.overwrite
				? clips.map((c) => (c.path === source.path ? result : c))
				: [result, ...clips.filter((c) => c.path !== result.path)];
			clipsChanged();
			const clip = present(result);
			return { clip, path: clip.path, fileName: clip.fileName, sizeBytes: clip.sizeBytes };
		},

		async getCameraVideo() {
			await latency();
			// No media files in the mock: the player shows a stand-in where the camera would be.
			return null;
		},

		async getTrackAudio(path) {
			await latency();
			if (!clips.some((c) => c.path === path)) throw new Error('That clip no longer exists.');
			// The mock has no media files, so there are no per-track audio files either.
			return [];
		},

		async getWaveforms(path) {
			await latency();
			const clip = clips.find((c) => c.path === path);
			if (!clip) throw new Error('That clip no longer exists.');
			// Made-up but stable outlines: loud bursts over a steady bed, per track.
			const sliceSecs = Math.max(0.01, clip.durationSecs / 8000);
			const count = Math.ceil(clip.durationSecs / sliceSecs);
			const tracks = clip.audioTracks.map((t) => {
				let seed = t.index * 7919 + clip.path.length * 31 + 1;
				const rand = () => (seed = (seed * 16807) % 2147483647) / 2147483647;
				const levels: number[] = [];
				let burst = 0;
				for (let i = 0; i < count; i++) {
					if (burst <= 0 && rand() < 0.004) burst = 40 + rand() * 200;
					const base = t.title === 'Microphone' ? 20 : 120 + 40 * Math.sin(i / 300);
					const value = burst > 0 ? 200 + rand() * 50 : base + rand() * 40;
					burst--;
					levels.push(Math.max(0, Math.min(255, Math.round(value))));
				}
				return levels;
			});
			return { sliceSecs, tracks };
		},

		async listAlbums() {
			await latency();
			const existing = new Set(clips.map((c) => c.path));
			return clone(albums.map((a) => ({ ...a, clipPaths: a.clipPaths.filter((p) => existing.has(p)) })));
		},

		async createAlbum(name) {
			await latency();
			const clean = name.trim();
			if (!clean) throw new Error('The album needs a name.');
			const album: Album = {
				id: `album-${Date.now().toString(36)}`,
				name: clean,
				hideFromClips: false,
				clipPaths: [],
				createdAt: new Date().toISOString(),
				builtin: false
			};
			albums = [...albums, album];
			albumsChanged();
			return clone(album);
		},

		async renameAlbum(id, name) {
			await latency();
			const clean = name.trim();
			if (!clean) throw new Error('The album needs a name.');
			if (id === 'favorites') throw new Error('Favorites is built in and keeps its name');
			return updateAlbum(id, (a) => ({ ...a, name: clean }));
		},

		async deleteAlbum(id) {
			await latency();
			if (id === 'favorites') throw new Error('Favorites is built in and cannot be deleted');
			findAlbum(id);
			albums = albums.filter((a) => a.id !== id);
			albumsChanged();
			clipsChanged();
		},

		async setAlbumHidden(id, hideFromClips) {
			await latency();
			return updateAlbum(id, (a) => ({ ...a, hideFromClips }));
		},

		async addToAlbum(id, paths) {
			await latency();
			const result = updateAlbum(id, (a) => ({ ...a, clipPaths: [...a.clipPaths, ...paths.filter((p) => !a.clipPaths.includes(p))] }));
			clipsChanged();
			return result;
		},

		async removeFromAlbum(id, paths) {
			await latency();
			const result = updateAlbum(id, (a) => ({ ...a, clipPaths: a.clipPaths.filter((p) => !paths.includes(p)) }));
			clipsChanged();
			return result;
		},

		async pickLogo() {
			await latency();
			return 'C:\\Users\\You\\Documents\\Umbra\\logos\\logo-1.png';
		},

		async fileUrl() {
			// A browser cannot read local files: a drawn stand-in.
			return MOCK_LOGO;
		},

		async pickProgram() {
			await latency();
			// A browser has no native file dialog; a sample program path stands in.
			return 'C:\\Games\\EA Games\\Battlefield V\\bfv.exe';
		},

		async pickFolder() {
			await latency();
			// A browser cannot open a native folder dialog, so the mock cycles through sample paths.
			const index = SAMPLE_FOLDERS.indexOf(settings.storage.folder);
			return SAMPLE_FOLDERS[(index + 1) % SAMPLE_FOLDERS.length];
		},

		onStatus: subscribe(statusHandlers),
		onClipSaved: subscribe(savedHandlers),
		onClipSaving: subscribe(new Set<(e: { game: string; waitSecs: number; lengthSecs: number }) => void>()),
		onClipFailed: subscribe(new Set<(e: { error: string }) => void>()),
		onExportProgress: subscribe(progressHandlers),
		onClipsChanged: subscribe(clipsChangedHandlers),
		onAlbumsChanged: subscribe(albumsChangedHandlers),
		onBenchmarkProgress: subscribe(benchmarkHandlers),
		onRecordingSaved: subscribe(recordingHandlers),
		onRecordingFinishing: subscribe(new Set<(e: RecordingFinishing) => void>()),
		async listProcessing() {
			// A long session being finished, a third of the way through.
			const started = Date.now() - 50_000;
			const total = 84e9;
			const done = Math.min(total, ((Date.now() - started) / 1000) * 570e6);
			return [
				{
					task: 'finish' as const,
					path: 'C:\\Users\\you\\Videos\\Umbra\\World of Warships\\World of Warships 2026-10-02 20-32-18 (session).mp4',
					fileName: 'World of Warships 2026-10-02 20-32-18 (session).mp4',
					game: 'World of Warships',
					kind: 'session' as const,
					bytesDone: done,
					bytesTotal: total,
					startedAt: new Date(started).toISOString(),
					etaSecs: (total - done) / 570e6
				}
			];
		},
		onProcessingStarted: subscribe(new Set<() => void>()),
		async shrinkEstimate() {
			return { files: 3, bytes: 240e9, afterBytes: 28e9, running: false };
		},
		async shrinkNow() {},
		async compressClips(paths) {
			return { queued: paths.length, waiting: false };
		},
		onShrinkResult: subscribe(new Set<(r: ShrinkResult) => void>()),
		async stopCompressing(paths) {
			return paths.length;
		},
		async burnIn(paths) {
			return paths.length;
		},
		async stopBurnIn(paths) {
			return paths.length;
		},
		onBurnResult: subscribe(new Set<(r: BurnResult) => void>()),
		onStorageCleaned: subscribe(new Set<(c: Cleanup) => void>()),
		onLowDisk: subscribe(new Set<(e: LowDisk) => void>()),
		async getAutoClipLog() {
			const t = Date.now() - 60_000;
			const entry = (ms: number, message: string, kind: AutoClipLogEntry['kind'] = null): AutoClipLogEntry => ({
				unixMs: t + ms,
				kind,
				confidence: kind ? 1 : null,
				source: kind ? 'cs2-gsi' : null,
				message
			});
			return [
				entry(0, 'CS2: listening on 127.0.0.1:47380'),
				entry(4_000, 'CS2 is connected'),
				entry(4_000, 'CS2: competitive on de_mirage'),
				entry(21_310, 'first event, waiting for more', 'kill'),
				entry(21_310, '2 events so far', 'headshot'),
				entry(38_870, '3 events so far', 'kill'),
				entry(38_870, '4 events so far', 'kill'),
				entry(38_870, '5 events so far', 'collateral'),
				entry(41_020, 'end of the round', 'roundDone'),
				entry(41_900, 'saved Counter-Strike 2 2026-11-18 21-04-11 - Triple kill, Collateral.mp4 (28.6 s, 5 events)'),
				entry(83_400, 'first event, waiting for more', 'kill'),
				entry(97_100, 'end of the round', 'roundDone'),
				entry(97_100, 'no clip of Kill: 1 of the 3 kills a clip needs')
			];
		},
		onAutoClipLog: subscribe(new Set<(e: AutoClipLogEntry) => void>()),
		async getInputTrack(path) {
			const clip = clips.find((c) => c.path === path);
			return clip?.input ? mockInputTrack(clip.durationSecs) : { blocks: [] };
		},
		async setInputCapture(on) {
			settings.input.enabled = on ?? !settings.input.enabled;
			persistSettings(settings);
			emitStatus();
		},
		async installCs2Config() {
			await latency();
			return 'C:\\Program Files (x86)\\Steam\\steamapps\\common\\Counter-Strike Global Offensive\\game\\csgo\\cfg\\gamestate_integration_umbra.cfg';
		},
		async getCs2Config() {
			const { port, token } = settings.autoClip.cs2;
			const text = ['"Umbra auto-clip"', '{', `    "uri"       "http://127.0.0.1:${port}/"`, '    "auth"', '    {', `        "token" "${token}"`, '    }', '}', ''];
			return { text: text.join('\n'), fileName: 'gamestate_integration_umbra.cfg', folder: null };
		},
		async pickVideos() {
			await latency();
			return ['C:\\Users\\you\\Videos\\Old clips\\Apex 2024.03.02 - 21.10.05.mp4'];
		},
		async listImportSources() {
			await latency();
			return [{ label: 'NVIDIA', path: 'C:\\Users\\you\\Videos\\NVIDIA', files: 3, bytes: 1.4 * 2 ** 30 }];
		},
		async scanImport(paths) {
			await latency();
			return paths.flatMap((p, i) => [
				{ path: `${p}\\Valorant 2025.07.24 - 01.41.38.01.mp4`, name: 'Valorant 2025.07.24 - 01.41.38.01.mp4', bytes: 380e6, game: 'Valorant', duplicate: false },
				{ path: `${p}\\Battlefield V 2025.12.17 - 23.27.58.01.mp4`, name: 'Battlefield V 2025.12.17 - 23.27.58.01.mp4', bytes: 520e6, game: 'Battlefield V', duplicate: i > 0 }
			]);
		},
		async startImport(files, moveOriginals) {
			await latency();
			void (async () => {
				const size = 4e9;
				const total = files.length * size * (moveOriginals ? 2 : 1);
				for (let i = 0; i < files.length; i++) {
					const name = files[i].path.split('\\').pop() ?? '';
					for (let step = 0; step <= 10; step++) {
						const checking = moveOriginals && step > 5;
						const fraction = checking ? (step - 5) / 5 : step / (moveOriginals ? 5 : 10);
						const bytesDone = i * size * (moveOriginals ? 2 : 1) + (checking ? size + fraction * size : fraction * size);
						importProgressHandlers.forEach((h) =>
							h({
								done: i,
								total: files.length,
								current: '',
								bytesDone,
								bytesTotal: total,
								rate: 1.4e9,
								etaSecs: step > 1 || i > 0 ? (total - bytesDone) / 1.4e9 : null,
								active: [{ name, bytes: size, stage: checking ? 'checking' : 'copying', fraction }],
								moving: moveOriginals
							})
						);
						await sleep(250);
					}
				}
				importDoneHandlers.forEach((h) => h({ imported: files.length, skipped: 0, removed: moveOriginals ? files.length : 0, failed: [] }));
			})();
		},
		onImportProgress: subscribe(importProgressHandlers),
		onImportDone: subscribe(importDoneHandlers),
		onFileDrop: subscribe(new Set<(e: FileDrop) => void>()),
		onLoadTestProgress: subscribe(loadHandlers)
	};
}
