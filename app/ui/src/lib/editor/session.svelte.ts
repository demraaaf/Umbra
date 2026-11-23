// One editing session per clip: trim points, crop, muted tracks, export state and the
// playback controller. Sessions outlive the editor screen, so leaving to the library and
// coming back keeps the work in progress.

import { beginExport, endExport, updateExport } from '$lib/stores/exports.svelte';
import {
	exportTarget,
	forcesMix,
	isAudioOnly,
	loadExportOptions,
	saveExportOptions,
	type CustomExport,
	type ExportOptions,
	type ExportPresetKey
} from './exportPresets';
import { toast } from 'svelte-sonner';
import { api, errorMessage } from '$lib/api';
import type {
	InputOverlay,
	CameraVideo,
	Clip,
	CropRect,
	ExportItem,
	ExportRequest,
	ExportResult,
	Fades,
	Placement,
	SpeedRange,
	Waveforms
} from '$lib/api/types';
import { DEFAULT_CAMERA_PLACEMENT, parseLogo, parsePlacement } from './overlay';
import { DEFAULT_INPUT_OVERLAY, InputTimeline, parseInputOverlay } from '$lib/input/overlay';
import { app, clipHooks, openClip, revealClip, upsertClip } from '$lib/stores/app.svelte';
import { Playback } from './playback.svelte';
import {
	aspectRatio,
	fitAspect,
	fullFrame,
	isFullFrame,
	moveRect,
	resizeRect,
	setField,
	snapRect,
	type AspectKey,
	type Handle
} from './crop';
import { formatBytes, formatDuration, clipTitle } from '$lib/format/format';

const round3 = (v: number) => Math.round(v * 1000) / 1000;

/** The mixer key of "Other apps" with the separately recorded apps taken out. */
const NO_APPS = -1;

/** A stretch of the clip, in seconds. */
export interface Range {
	start: number;
	end: number;
}

const MAGNET_KEY = 'umbra.timeline-magnet';

function loadMagnet(): boolean {
	try {
		return localStorage.getItem(MAGNET_KEY) !== '0';
	} catch {
		return true;
	}
}

/** The timeline edits, as undo keeps them. */
interface Snapshot {
	inSecs: number;
	outSecs: number;
	cuts: Range[];
	splits: number[];
	speeds: SpeedRange[];
	cameraHidden: Range[];
	cameraPlacement: Placement | null;
	cameraOff: boolean;
	logoPlacement: Placement | null;
	logoOff: boolean;
	inputPlacement: Placement | null;
	inputOff: boolean;
}

/** Speeds offered in the editor, slowest first. */
export const SPEEDS = [0.25, 0.5, 0.75, 1, 1.5, 2, 4] as const;

export function speedLabel(speed: number): string {
	return `${speed.toLocaleString('en-US', { maximumFractionDigits: 2 })}×`;
}

/** What the export will be, for the summary under the button and in the settings. */
export interface ExportInfo {
	secs: number;
	width: number;
	height: number;
	fps: number;
	codec: string;
	tracks: number;
	/** Expected size in bytes, or the limit when `limit` is set. */
	bytes: number;
	limit: boolean;
}

/** Joins overlapping and touching ranges; the result is sorted. */
function mergeRanges(ranges: Range[]): Range[] {
	const merged: Range[] = [];
	for (const c of [...ranges].sort((a, b) => a.start - b.start)) {
		const last = merged[merged.length - 1];
		if (last && c.start <= last.end + 0.0005) last.end = Math.max(last.end, c.end);
		else merged.push({ ...c });
	}
	return merged;
}

export class EditSession {
	clip = $state<Clip>() as Clip;
	readonly playback: Playback;

	inSecs = $state(0);
	outSecs = $state(0);

	cropEnabled = $state(false);
	crop = $state<CropRect>({ x: 0, y: 0, width: 0, height: 0 });
	aspect = $state<AspectKey>('free');

	/** Timeline zoom: the visible window. `viewSpan` 0 means the whole clip is visible. */
	viewStart = $state(0);
	viewSpan = $state(0);

	/** Track volume by track index, 0 to 2 (0 to 200 %). A missing entry means 1. */
	trackVolumes = $state<Record<number, number>>({});
	/** Tracks switched off with their mute button; their volume is kept for when they return. */
	mutedTracks = $state<number[]>([]);
	overwrite = $state(false);
	/** How the export is written (see exportPresets.ts); remembered for the next clip. */
	exportOptions = $state<ExportOptions>(loadExportOptions());
	/** The exported file's name without .mp4; empty uses the clip's name with "(edit)". */
	exportName = $state('');

	/** The range picked with Shift and the arrow keys or Shift and a drag; null when none. */
	selection = $state<Range | null>(null);
	/** Where the selection was started; the other end follows the playhead. */
	#anchor = 0;
	/** Parts left out of the export, sorted and never overlapping, in seconds. */
	cuts = $state<Range[]>([]);
	/** Cut points set with C, sorted. The parts between them go with Delete, Q and W. */
	splits = $state<number[]>([]);
	/** Stretches played at another speed, sorted and never overlapping, in clip seconds. */
	speeds = $state<SpeedRange[]>([]);
	/** Fades of the whole edit; the first clip's session holds them. */
	fades = $state<Fades>({ fadeIn: 0, fadeOut: 0, between: 0.5 });

	/** Undo and redo of the timeline edits (trim, cut points, parts cut out). */
	#past: Snapshot[] = [];
	#future: Snapshot[] = [];
	#lastKind = '';
	#lastAt = 0;
	undoDepth = $state(0);
	redoDepth = $state(0);

	/** Drags on the timeline snap to cut points, bookmarks, trim ends and the playhead. */
	magnet = $state(loadMagnet());

	/** Apps whose copy inside "Other apps" was found, so their volume applies there too. */
	unmixedApps = $state<number[]>([]);

	/** Waveform outlines, loaded the first time the timeline has room to show them. */
	waves = $state<Waveforms | null>(null);
	wavesState = $state<'idle' | 'loading' | 'ready' | 'error'>('idle');

	exportState = $state<'idle' | 'running' | 'done' | 'error'>('idle');
	exportFraction = $state(0);
	exportError = $state<string | null>(null);
	exportedClip = $state<Clip | null>(null);
	/** The last export's file, whatever its format. */
	exportedFile = $state<ExportResult | null>(null);

	/** Where the camera shows, relative to the exported picture; null for a clip without one. */
	cameraPlacement = $state<Placement | null>(null);
	/** Left out of the export. */
	cameraOff = $state(false);
	/** Stretches where the camera is hidden, sorted and never overlapping, in clip seconds. */
	cameraHidden = $state<Range[]>([]);
	/** The camera as a file of its own, for the preview; null until it is ready. */
	cameraVideo = $state<CameraVideo | null>(null);
	/** The picture the clip was saved with, and where it shows; null for none. */
	logo = $state<{ path: string; placement: Placement } | null>(null);
	logoOff = $state(false);
	/** How the clip's keyboard and mouse input shows; null for a clip without input. */
	input = $state<InputOverlay | null>(null);
	inputOff = $state(false);
	/** The clip's input, for the preview; null until loaded. */
	inputTimeline = $state<InputTimeline | null>(null);
	/** Which one the stage's resize handles are on. */
	overlaySelected = $state<'camera' | 'logo' | 'input' | null>(null);

	constructor(clip: Clip) {
		this.clip = clip;
		this.playback = new Playback(clip.videoUrl, clip.durationSecs, clip.fps);
		this.outSecs = clip.durationSecs;
		this.crop = fullFrame(clip.width, clip.height);
		this.playback.setRange(this.inSecs, this.outSecs);
		void this.#loadTrackAudio();
		if (clip.camera) {
			this.cameraPlacement = parsePlacement(clip.details.camera) ?? { ...DEFAULT_CAMERA_PLACEMENT };
			void this.#loadCamera();
		}
		this.logo = parseLogo(clip.details.logo);
		if (clip.input) {
			this.input = parseInputOverlay(clip.details.input, app.settings?.input.overlay ?? DEFAULT_INPUT_OVERLAY);
			void this.#loadInput();
		}
	}

	async #loadInput() {
		try {
			this.inputTimeline = new InputTimeline(await api.getInputTrack(this.clip.path));
		} catch {
			this.inputTimeline = null;
		}
	}

	async #loadCamera() {
		if (this.playback.simulated) return;
		try {
			this.cameraVideo = await api.getCameraVideo(this.clip.path);
		} catch {
			this.cameraVideo = null;
		}
	}

	// ---------- Camera and logo ----------

	/** The camera's picture: its width over its height. */
	get cameraAspect(): number {
		return this.clip.camera ? this.clip.camera.width / this.clip.camera.height : 16 / 9;
	}

	/** The camera or the logo is drawn into the export, so it has to be encoded again. */
	get overlaysDrawn(): boolean {
		return (
			(this.cameraPlacement !== null && !this.cameraOff) ||
			(this.logo !== null && !this.logoOff) ||
			(this.input !== null && !this.inputOff)
		);
	}

	/** The camera filmed at `t` and is not hidden there. */
	cameraShownAt(t: number): boolean {
		if (!this.cameraPlacement || this.cameraOff) return false;
		if (this.cameraHidden.some((r) => t >= r.start && t < r.end)) return false;
		const stretches = this.cameraVideo?.stretches;
		// The mock has no camera file; the stand-in shows throughout.
		if (!stretches) return this.playback.simulated;
		return stretches.some(([a, b]) => t >= a && t < b);
	}

	setCameraPlacement(placement: Placement) {
		this.#remember('camera-place');
		this.cameraPlacement = placement;
	}

	setLogoPlacement(placement: Placement) {
		if (!this.logo) return;
		this.#remember('logo-place');
		this.logo = { ...this.logo, placement };
	}

	setInputPlacement(placement: Placement) {
		if (!this.input) return;
		this.#remember('input-place');
		this.input = { ...this.input, placement };
	}

	setInputOff(off: boolean) {
		this.#remember();
		this.inputOff = off;
	}

	setCameraOff(off: boolean) {
		this.#remember();
		this.cameraOff = off;
	}

	setLogoOff(off: boolean) {
		this.#remember();
		this.logoOff = off;
	}

	/** Hides the camera from `start` to `end`. */
	hideCamera(start: number, end: number) {
		if (end - start < 0.0005) return;
		this.#remember();
		this.cameraHidden = mergeRanges([...this.cameraHidden, { start, end }]);
	}

	/** Shows the camera again where it is hidden at `t`. */
	showCameraAt(t: number) {
		const kept = this.cameraHidden.filter((r) => !(t >= r.start && t < r.end));
		if (kept.length === this.cameraHidden.length) return;
		this.#remember();
		this.cameraHidden = kept;
	}

	/** Where the camera was saved to show, with this clip's crop and look. */
	resetCameraPlacement() {
		if (!this.clip.camera) return;
		this.#remember();
		this.cameraPlacement = parsePlacement(this.clip.details.camera) ?? { ...DEFAULT_CAMERA_PLACEMENT };
	}

	/** Separate track files make per-track volume audible in the preview; without them it falls back. */
	async #loadTrackAudio() {
		if (this.playback.simulated || this.clip.audioTracks.length === 0) {
			if (!this.playback.simulated) this.playback.audioMode = 'video';
			return;
		}
		try {
			const files = await api.getTrackAudio(this.clip.path);
			const unmixed = files.find((f) => f.kind === 'othersWithoutApps');
			this.unmixedApps = unmixed?.apps ?? [];
			// The mixer keys files by index; "Other apps without the apps" gets its own key.
			const tracks = files.map((f) => (f.kind === 'othersWithoutApps' ? { ...f, index: NO_APPS } : f));
			await this.playback.useTracks(tracks);
			this.playback.setTrackVolumes(this.#previewGains());
		} catch {
			this.playback.audioMode = 'video';
		}
	}

	// ---------- Derived state ----------

	get frameSecs(): number {
		return 1 / this.clip.fps;
	}
	get duration(): number {
		return this.playback.duration;
	}
	/** Length of the export: the trimmed range without the parts cut out. */
	get lengthSecs(): number {
		const removed = this.keptCuts.reduce((sum, c) => sum + (c.end - c.start), 0);
		return Math.max(0, this.outSecs - this.inSecs - removed);
	}
	/** The cuts that fall inside the trimmed range, clipped to it. */
	get keptCuts(): Range[] {
		return this.cuts
			.map((c) => ({ start: Math.max(c.start, this.inSecs), end: Math.min(c.end, this.outSecs) }))
			.filter((c) => c.end - c.start > 0.0005);
	}
	/** Length of the export after speed changes: what the viewer sees. */
	get outputSecs(): number {
		return this.outputTimeOf(this.outSecs);
	}

	/** Where clip time `t` falls in this clip's part of the export (0 at the trim start). */
	outputTimeOf(t: number): number {
		const end = Math.min(Math.max(t, this.inSecs), this.outSecs);
		const cuts = this.keptCuts;
		const edges = new Set<number>([this.inSecs, end]);
		for (const c of cuts) [c.start, c.end].forEach((x) => x > this.inSecs && x < end && edges.add(x));
		for (const r of this.speeds) [r.start, r.end].forEach((x) => x > this.inSecs && x < end && edges.add(x));
		const sorted = [...edges].sort((a, b) => a - b);
		let out = 0;
		for (let i = 0; i < sorted.length - 1; i++) {
			const [a, b] = [sorted[i], sorted[i + 1]];
			const mid = (a + b) / 2;
			if (cuts.some((c) => mid >= c.start && mid < c.end)) continue;
			out += (b - a) / (this.speeds.find((r) => mid >= r.start && mid < r.end)?.speed ?? 1);
		}
		return out;
	}

	/** Seconds the arrow keys skip, from the settings. */
	get seekSecs(): number {
		return app.settings?.player.seekSecs ?? 5;
	}
	get trimmed(): boolean {
		return this.inSecs > 0.0005 || this.outSecs < this.duration - 0.0005;
	}
	get cropActive(): boolean {
		return this.cropEnabled && !isFullFrame(this.crop, this.clip.width, this.clip.height);
	}
	get audioChanged(): boolean {
		return this.clip.audioTracks.some((t) => this.effectiveVolume(t.index) !== 1);
	}
	get hasEdits(): boolean {
		return (
			this.trimmed ||
			this.cropActive ||
			this.audioChanged ||
			this.keptCuts.length > 0 ||
			this.speeds.length > 0 ||
			this.fades.fadeIn > 0 ||
			this.fades.fadeOut > 0
		);
	}
	/**
	 * The packets can be copied (Original, MP4, one clip, no crop, speed or fades), so cutting
	 * on keyframes instead of exact frames saves re-encoding the whole video.
	 */
	get copyable(): boolean {
		return (
			exportTarget(this.exportOptions) === null &&
			this.exportOptions.format === 'mp4' &&
			!this.cropActive &&
			this.speeds.every((s) => Math.abs(s.speed - 1) < 1e-6) &&
			this.fades.fadeIn === 0 &&
			this.fades.fadeOut === 0 &&
			!this.overlaysDrawn
		);
	}
	/** Something to export: an edit, a camera or logo to draw in, or settings that change the file. */
	get canExport(): boolean {
		return (
			(this.hasEdits || this.overlaysDrawn || exportTarget(this.exportOptions) !== null) &&
			!this.busy &&
			this.lengthSecs > 0
		);
	}

	/** Picks an export preset; the phone one starts a 9:16 crop from the middle to move. */
	setExportPreset(key: ExportPresetKey) {
		this.setExport({ preset: key });
		if (key === 'phone') {
			this.setAspect('9:16');
			this.setCropEnabled(true);
		}
	}

	/** Changes the export settings and remembers them for the next export. */
	setExport(change: Partial<ExportOptions>) {
		this.exportOptions = { ...this.exportOptions, ...change };
		saveExportOptions($state.snapshot(this.exportOptions) as ExportOptions);
	}

	setCustomExport(change: Partial<CustomExport>) {
		this.setExport({ custom: { ...this.exportOptions.custom, ...change } });
	}

	/** What the export will be: length, picture, frame rate, codec, tracks and about how big. */
	get exportInfo(): ExportInfo {
		return this.exportInfoFor(this.outputSecs, 1);
	}

	/** Like `exportInfo`, for an edit `secs` long made of `clips` clips. */
	exportInfoFor(secs: number, clips: number): ExportInfo {
		const target = exportTarget(this.exportOptions);
		const format = this.exportOptions.format;
		const audioKbps0 = app.settings?.audio.bitrateKbps ?? 160;
		if (isAudioOnly(format)) {
			const kbps = format === 'wav' ? 1536 : format === 'flac' ? 800 : Math.max(128, audioKbps0);
			return { secs, width: 0, height: 0, fps: 0, codec: format.toUpperCase(), tracks: 1, bytes: (kbps * 1000 * secs) / 8, limit: false };
		}
		if (format === 'gif') {
			const w = this.exportOptions.gifWidth;
			const sw = this.cropActive ? this.crop.width : this.clip.width;
			const sh = this.cropActive ? this.crop.height : this.clip.height;
			const h = Math.round((sh * w) / Math.max(1, sw) / 2) * 2;
			const fps = target?.maxFps ?? 15;
			// Roughly what a palette GIF of game footage takes per pixel and frame.
			return { secs, width: w, height: h, fps, codec: 'GIF', tracks: -1, bytes: w * h * fps * secs * 0.65, limit: false };
		}
		let width = this.cropActive ? this.crop.width : this.clip.width;
		let height = this.cropActive ? this.crop.height : this.clip.height;
		if (target?.maxHeight && target.maxHeight < height) {
			width = Math.max(2, Math.floor((width * target.maxHeight) / height / 2) * 2);
			height = Math.floor(target.maxHeight / 2) * 2;
		}
		const fps = target?.maxFps ? Math.min(target.maxFps, this.clip.fps) : this.clip.fps;
		const kept = this.keptTracks.length;
		const tracks = (target?.mixOnly || clips > 1) && kept > 0 ? 1 : kept;
		const audioKbps = app.settings?.audio.bitrateKbps ?? 160;
		let bytes: number;
		let limit = false;
		if (target?.maxBytes) {
			bytes = target.maxBytes;
			limit = true;
		} else if (target?.videoKbps) {
			bytes = ((target.videoKbps + tracks * audioKbps) * 1000 * secs) / 8;
		} else {
			// The export keeps the source's bits per pixel: fewer pixels or frames, fewer bytes
			// (the frame rate counts for at least 35 %, as in the encoder).
			const perSec = this.clip.durationSecs > 0 ? this.clip.sizeBytes / this.clip.durationSecs : 0;
			const area = (width * height) / Math.max(1, this.clip.width * this.clip.height);
			const frames = Math.max(0.35, fps / Math.max(1, this.clip.fps));
			bytes = perSec * secs * area * frames;
		}
		return { secs, width, height, fps, codec: (target?.codec ?? this.clip.codec).toUpperCase(), tracks, bytes, limit };
	}

	/** True when the chosen preset or format writes one mixed track whatever the audio choice says. */
	get exportForcesMix(): boolean {
		return forcesMix(this.exportOptions.preset, this.exportOptions.format);
	}

	setFades(change: Partial<Fades>) {
		this.fades = { ...this.fades, ...change };
	}
	get busy(): boolean {
		return this.exportState === 'running';
	}
	/** Tracks that end up in the export, with their volume. 0 % leaves a track out. */
	get keptTracks(): { index: number; volume: number }[] {
		return this.clip.audioTracks
			.map((t) => ({ index: t.index, volume: Math.round(this.effectiveVolume(t.index) * 100) / 100 }))
			.filter((t) => t.volume > 0);
	}
	/** Why the preview may not reflect track volumes, or null when it does. */
	get previewNote(): string | null {
		switch (this.playback.audioMode) {
			case 'none':
				return 'The mock has no audio files. Volumes apply to the export.';
			case 'pending':
				return 'Preparing the per-track preview.';
			case 'video':
				return 'Per-track preview is unavailable for this clip, so the preview plays the recorded mix. Volumes still apply to the export.';
			default:
				return null;
		}
	}

	// ---------- Timeline view ----------

	get visibleSpan(): number {
		return this.viewSpan > 0 ? Math.min(this.viewSpan, this.duration) : this.duration;
	}
	get visibleStart(): number {
		return Math.min(Math.max(0, this.viewStart), Math.max(0, this.duration - this.visibleSpan));
	}
	get zoomed(): boolean {
		return this.visibleSpan < this.duration - 0.001;
	}

	/** Zooms by `factor` (above 1 zooms in) keeping `anchor` (seconds) at the same screen position. */
	zoomAt(factor: number, anchor: number) {
		const span = this.visibleSpan;
		const next = Math.min(this.duration, Math.max(Math.min(this.duration, 0.5), span / factor));
		if (next >= this.duration - 0.001) {
			this.fitView();
			return;
		}
		const rel = span > 0 ? (anchor - this.visibleStart) / span : 0;
		this.viewSpan = next;
		this.viewStart = Math.min(this.duration - next, Math.max(0, anchor - rel * next));
	}

	panBy(seconds: number) {
		this.viewStart = Math.min(this.duration - this.visibleSpan, Math.max(0, this.visibleStart + seconds));
	}

	fitView() {
		this.viewSpan = 0;
		this.viewStart = 0;
	}

	// ---------- Trim ----------

	#snap(seconds: number): number {
		return round3(Math.round(seconds * this.clip.fps) / this.clip.fps);
	}

	setIn(seconds: number) {
		this.#remember('in');
		const max = Math.max(0, this.outSecs - this.frameSecs);
		this.inSecs = Math.min(max, Math.max(0, this.#snap(seconds)));
		this.#syncPlayback();
	}

	setOut(seconds: number) {
		this.#remember('out');
		const min = Math.min(this.duration, this.inSecs + this.frameSecs);
		this.outSecs = Math.max(min, Math.min(this.duration, this.#snap(seconds)));
		this.#syncPlayback();
	}

	setInAtPlayhead() {
		this.setIn(this.playback.time);
	}
	setOutAtPlayhead() {
		this.setOut(this.playback.time);
	}
	resetTrim() {
		this.#remember();
		this.inSecs = 0;
		this.outSecs = this.duration;
		this.#syncPlayback();
	}

	#syncPlayback() {
		this.playback.setRange(this.inSecs, this.outSecs);
		this.playback.setSkips(this.keptCuts);
		this.playback.setSpeeds(this.speeds);
	}

	// ---------- Selection and cuts ----------

	/**
	 * Moves the playhead to `seconds` and stretches the selection from its anchor to there.
	 * A new selection starts at the playhead, unless the playhead sits on an end of the
	 * current one, which then keeps growing or shrinking from its other end.
	 */
	selectTo(seconds: number) {
		const time = this.playback.time;
		const half = this.frameSecs / 2;
		const sel = this.selection;
		if (!sel) this.#anchor = this.#snap(time);
		else if (Math.abs(time - sel.end) <= half) this.#anchor = sel.start;
		else if (Math.abs(time - sel.start) <= half) this.#anchor = sel.end;
		else this.#anchor = this.#snap(time);
		this.playback.pause();
		this.dragSelection(seconds);
	}

	/** Shift and an arrow key: grow or shrink the selection by `seconds`. */
	selectBy(seconds: number) {
		this.selectTo(this.playback.time + seconds);
	}

	/** Starts a selection at `seconds`, for Shift and a drag on the timeline. */
	beginSelection(seconds: number) {
		this.playback.pause();
		this.#anchor = this.#snap(Math.min(this.duration, Math.max(0, seconds)));
		this.selection = null;
		this.playback.seek(this.#anchor);
	}

	/** Moves the free end of the selection (and the playhead) to `seconds`. */
	dragSelection(seconds: number) {
		const target = Math.min(this.duration, Math.max(0, this.#snap(seconds)));
		this.playback.seek(target);
		const start = Math.min(this.#anchor, target);
		const end = Math.max(this.#anchor, target);
		this.selection = end - start >= this.frameSecs / 2 ? { start, end } : null;
	}

	clearSelection() {
		this.selection = null;
	}

	/** Leaves the selected part out of the export. */
	cutSelection() {
		const sel = this.selection;
		if (!sel || this.busy) return;
		const before = this.cuts;
		this.#remember();
		this.cuts = mergeRanges([...this.cuts, sel]);
		if (this.lengthSecs <= 0.0005) {
			// Nothing would be left to export.
			this.cuts = before;
			this.#dropLast();
			toast.error('That would cut out the whole clip');
			return;
		}
		this.selection = null;
		this.#syncPlayback();
		this.playback.seek(Math.min(sel.end, this.duration));
	}

	/** Makes the selection the whole export: the trim moves to its ends. */
	keepSelection() {
		const sel = this.selection;
		if (!sel || this.busy) return;
		this.#remember();
		this.inSecs = sel.start;
		this.outSecs = sel.end;
		this.selection = null;
		this.#syncPlayback();
		this.playback.seek(sel.start);
	}

	/** Puts a cut-out part back. */
	restoreCut(index: number) {
		this.#remember();
		this.cuts = this.cuts.filter((_, i) => i !== index);
		this.#syncPlayback();
	}

	clearCuts() {
		this.#remember();
		this.cuts = [];
		this.#syncPlayback();
	}

	// ---------- Cut points ----------

	/** Cut points inside the trimmed range. */
	get innerSplits(): number[] {
		const half = this.frameSecs / 2;
		return this.splits.filter((t) => t > this.inSecs + half && t < this.outSecs - half);
	}

	/** The part between cut points (or the trim ends) that holds the playhead, when there are cut points. */
	get segment(): Range | null {
		const inner = this.innerSplits;
		if (inner.length === 0) return null;
		const bounds = [this.inSecs, ...inner, this.outSecs];
		const t = Math.min(Math.max(this.playback.time, this.inSecs), this.outSecs);
		for (let i = 0; i < bounds.length - 1; i++) {
			if (t < bounds[i + 1] - 1e-6 || i === bounds.length - 2) return { start: bounds[i], end: bounds[i + 1] };
		}
		return null;
	}

	/** C: sets a cut point at the playhead, or takes away the one there. */
	toggleCutAtPlayhead() {
		if (this.busy) return;
		const t = this.#snap(this.playback.time);
		const half = this.frameSecs / 2;
		const near = this.splits.findIndex((s) => Math.abs(s - t) <= half);
		if (near >= 0) {
			this.#remember();
			this.splits = this.splits.filter((_, i) => i !== near);
			return;
		}
		if (t <= this.inSecs + half || t >= this.outSecs - half) {
			toast('Cut points go between the start and the end of the trim');
			return;
		}
		this.#remember();
		this.splits = [...this.splits, t].sort((a, b) => a - b);
	}

	/** Moves a cut point (dragged on the timeline), never past its neighbours. */
	moveSplit(index: number, seconds: number) {
		const half = this.frameSecs / 2;
		const low = (this.splits[index - 1] ?? this.inSecs) + half * 2;
		const high = (this.splits[index + 1] ?? this.outSecs) - half * 2;
		if (high <= low) return;
		this.#remember('split');
		const next = [...this.splits];
		next[index] = Math.min(high, Math.max(low, this.#snap(seconds)));
		this.splits = next;
	}

	clearSplits() {
		if (this.splits.length === 0) return;
		this.#remember();
		this.splits = [];
	}

	/** Delete: takes out the part between cut points under the playhead. False when there is none. */
	deleteSegment(): boolean {
		const seg = this.segment;
		if (!seg || this.busy) return false;
		const half = this.frameSecs / 2;
		const atStart = seg.start <= this.inSecs + half;
		const atEnd = seg.end >= this.outSecs - half;
		if (atStart && atEnd) return false;
		this.#remember();
		if (atStart) this.inSecs = seg.end;
		else if (atEnd) this.outSecs = seg.start;
		else this.cuts = mergeRanges([...this.cuts, seg]);
		if (this.lengthSecs <= 0.0005) {
			this.undo();
			this.#future.pop();
			this.#counts();
			toast.error('That would cut out the whole clip');
			return true;
		}
		this.#syncPlayback();
		this.playback.seek(atEnd ? this.outSecs : seg.end);
		return true;
	}

	/** Q: removes everything before the playhead's part (or before the playhead, without cut points). */
	deleteBefore() {
		if (this.busy) return;
		const t = this.playback.time;
		const half = this.frameSecs / 2;
		const prev = this.innerSplits.filter((s) => s <= t + half).pop();
		const at = prev ?? this.#snap(t);
		if (at <= this.inSecs + half) return;
		// A change of its own for undo, not part of a handle drag just before.
		this.#lastKind = '';
		this.setIn(at);
		this.#lastKind = '';
		this.playback.seek(this.inSecs);
	}

	/** W: removes everything after the playhead's part (or after the playhead, without cut points). */
	deleteAfter() {
		if (this.busy) return;
		const t = this.playback.time;
		const half = this.frameSecs / 2;
		const next = this.innerSplits.find((s) => s > t + half);
		const at = next ?? this.#snap(t);
		if (at >= this.outSecs - half) return;
		// A change of its own for undo, not part of a handle drag just before.
		this.#lastKind = '';
		this.setOut(at);
		this.#lastKind = '';
		this.playback.seek(Math.max(this.inSecs, this.outSecs - this.frameSecs));
	}

	toggleMagnet() {
		this.magnet = !this.magnet;
		try {
			localStorage.setItem(MAGNET_KEY, this.magnet ? '1' : '0');
		} catch {
			// Only a convenience.
		}
	}

	// ---------- Speed ----------

	/** What a speed change applies to: the selection, else the part between cut points
	 * under the playhead, else (to go back to normal) the sped-up or slowed stretch there. */
	speedTarget(forNormal = false): Range | null {
		if (this.selection) return this.selection;
		if (this.segment) return this.segment;
		if (forNormal) {
			const t = this.playback.time;
			const r = this.speeds.find((x) => t >= x.start && t < x.end);
			if (r) return { start: r.start, end: r.end };
		}
		return null;
	}

	/** Plays `range` at `speed` (1 back to normal), replacing what was there. */
	setSpeed(range: Range, speed: number) {
		if (this.busy) return;
		this.#remember();
		const half = this.frameSecs / 2;
		const kept: SpeedRange[] = [];
		for (const r of this.speeds) {
			if (r.end <= range.start || r.start >= range.end) {
				kept.push(r);
				continue;
			}
			if (r.start < range.start) kept.push({ ...r, end: range.start });
			if (r.end > range.end) kept.push({ ...r, start: range.end });
		}
		if (Math.abs(speed - 1) > 1e-6) kept.push({ start: round3(range.start), end: round3(range.end), speed });
		this.speeds = kept.filter((r) => r.end - r.start > half).sort((a, b) => a.start - b.start);
		this.selection = null;
		this.#syncPlayback();
	}

	/** S, F and N: slow motion, fast and normal for the selection or the part under the playhead. */
	speedHere(speed: number): boolean {
		const range = this.speedTarget(speed === 1);
		if (!range) {
			toast('Select a part first', {
				description: 'Shift and drag on the timeline, or set cut points with C, then pick a speed.'
			});
			return false;
		}
		this.setSpeed(range, speed);
		return true;
	}

	clearSpeeds() {
		if (this.speeds.length === 0) return;
		this.#remember();
		this.speeds = [];
		this.#syncPlayback();
	}

	// ---------- Undo ----------

	#snapshot(): Snapshot {
		return {
			inSecs: this.inSecs,
			outSecs: this.outSecs,
			cuts: this.cuts.map((c) => ({ ...c })),
			splits: [...this.splits],
			speeds: this.speeds.map((r) => ({ ...r })),
			cameraHidden: this.cameraHidden.map((r) => ({ ...r })),
			cameraPlacement: this.cameraPlacement ? { ...this.cameraPlacement } : null,
			cameraOff: this.cameraOff,
			logoPlacement: this.logo ? { ...this.logo.placement } : null,
			logoOff: this.logoOff,
			inputPlacement: this.input ? { ...this.input.placement } : null,
			inputOff: this.inputOff
		};
	}

	/**
	 * Keeps the timeline edits as they are before a change. Changes of the same `kind` close
	 * together (dragging a handle) count as one.
	 */
	#remember(kind = '') {
		const now = performance.now();
		if (kind && kind === this.#lastKind && now - this.#lastAt < 1000) {
			this.#lastAt = now;
			return;
		}
		this.#lastKind = kind;
		this.#lastAt = now;
		this.#past.push(this.#snapshot());
		if (this.#past.length > 200) this.#past.shift();
		this.#future = [];
		this.#counts();
	}

	/** Forgets the last remembered state, for a change that did not happen after all. */
	#dropLast() {
		this.#past.pop();
		this.#counts();
	}

	#restore(state: Snapshot) {
		this.inSecs = state.inSecs;
		this.outSecs = state.outSecs;
		this.cuts = state.cuts;
		this.splits = state.splits;
		this.speeds = state.speeds;
		this.cameraHidden = state.cameraHidden;
		this.cameraPlacement = state.cameraPlacement;
		this.cameraOff = state.cameraOff;
		if (this.logo && state.logoPlacement) this.logo = { ...this.logo, placement: state.logoPlacement };
		this.logoOff = state.logoOff;
		if (this.input && state.inputPlacement) this.input = { ...this.input, placement: state.inputPlacement };
		this.inputOff = state.inputOff;
		this.selection = null;
		this.#lastKind = '';
		this.#syncPlayback();
	}

	#counts() {
		this.undoDepth = this.#past.length;
		this.redoDepth = this.#future.length;
	}

	undo(): boolean {
		const state = this.#past.pop();
		if (!state || this.busy) return false;
		this.#future.push(this.#snapshot());
		this.#restore(state);
		this.#counts();
		return true;
	}

	redo(): boolean {
		const state = this.#future.pop();
		if (!state || this.busy) return false;
		this.#past.push(this.#snapshot());
		this.#restore(state);
		this.#counts();
		return true;
	}

	// ---------- Waveforms ----------

	async loadWaves() {
		if (this.wavesState !== 'idle' || this.clip.audioTracks.length === 0) return;
		this.wavesState = 'loading';
		try {
			this.waves = await api.getWaveforms(this.clip.path);
			this.wavesState = 'ready';
		} catch {
			this.wavesState = 'error';
		}
	}

	/** The webview reported the real media length; follow it if the untrimmed end was in use. */
	onDuration(duration: number) {
		if (Math.abs(this.outSecs - this.clip.durationSecs) < 0.001 || this.outSecs > duration) {
			this.outSecs = duration;
		}
		this.#syncPlayback();
	}

	// ---------- Tracks ----------

	isMuted(index: number): boolean {
		return this.mutedTracks.includes(index);
	}

	/** The slider value, 0 to 2, regardless of mute. */
	volumeOf(index: number): number {
		return this.trackVolumes[index] ?? 1;
	}

	/** What the export and the preview use: 0 when muted. */
	effectiveVolume(index: number): number {
		return this.isMuted(index) ? 0 : this.volumeOf(index);
	}

	/**
	 * Gain of each track file in the preview. With a combined track, the preview plays it the
	 * way the export rebuilds it: the tracks in it at their volumes, times its own volume,
	 * plus the tracks that are only on their own (the microphone). An app that older clips
	 * also have inside "Other apps" is played on its own only when it could be taken out of
	 * "Other apps", so nothing is heard twice. Without a combined track every track plays.
	 */
	#previewGains(): Record<number, number> {
		const tracks = this.clip.audioTracks;
		const mix = tracks.find((t) => t.role === 'mix');
		if (!mix) return Object.fromEntries(tracks.map((t) => [t.index, this.effectiveVolume(t.index)]));
		// A combined track that is left out still previews its parts, so the clip is not silent.
		const master = this.effectiveVolume(mix.index) || 1;
		const others = tracks.find((t) => t.title.toLowerCase() === 'other apps');
		const unmixed = others && this.unmixedApps.length > 0;
		const gains: Record<number, number> = {};
		for (const t of tracks) {
			const volume = this.effectiveVolume(t.index) * master;
			if (unmixed && t.index === others.index) {
				// Played as "Other apps without the apps", next to each app at its own volume.
				gains[t.index] = 0;
				gains[NO_APPS] = volume;
			} else if (t.role === 'inMix' || t.role === 'separate' || (unmixed && this.unmixedApps.includes(t.index))) {
				// Tracks only on their own (the microphone) are heard too, though the export
				// keeps them out of Mix.
				gains[t.index] = volume;
			} else {
				gains[t.index] = 0;
			}
		}
		return gains;
	}

	/** True when this app's volume also reaches "Other apps" and Mix. */
	isUnmixed(index: number): boolean {
		return this.unmixedApps.includes(index);
	}

	setTrackVolume(index: number, volume: number) {
		const v = Math.min(2, Math.max(0, Math.round(volume * 100) / 100));
		this.trackVolumes = { ...this.trackVolumes, [index]: v };
		// Raising a muted track's volume brings it back.
		if (v > 0 && this.isMuted(index)) this.mutedTracks = this.mutedTracks.filter((i) => i !== index);
		this.playback.setTrackVolumes(this.#previewGains());
	}

	/** Mute a track that is heard; bring back a silent one (muted or pulled to 0 %, which returns at 100 %). */
	toggleTrack(index: number) {
		if (this.effectiveVolume(index) > 0) {
			this.mutedTracks = [...this.mutedTracks, index];
		} else {
			this.mutedTracks = this.mutedTracks.filter((i) => i !== index);
			if (this.volumeOf(index) === 0) this.trackVolumes = { ...this.trackVolumes, [index]: 1 };
		}
		this.playback.setTrackVolumes(this.#previewGains());
	}

	resetTracks() {
		this.trackVolumes = {};
		this.mutedTracks = [];
		this.playback.setTrackVolumes(this.#previewGains());
	}

	// ---------- Crop ----------

	setCropEnabled(enabled: boolean) {
		this.cropEnabled = enabled;
		if (enabled) {
			const ratio = aspectRatio(this.aspect);
			if (ratio) this.crop = fitAspect(this.crop, ratio, this.clip.width, this.clip.height);
		}
	}

	setAspect(key: AspectKey) {
		this.aspect = key;
		const ratio = aspectRatio(key);
		if (ratio) this.crop = fitAspect(this.crop, ratio, this.clip.width, this.clip.height);
	}

	moveCrop(dx: number, dy: number) {
		this.crop = moveRect(this.crop, dx, dy, this.clip.width, this.clip.height);
	}

	dragCrop(start: CropRect, handle: Handle | 'move', dx: number, dy: number) {
		const { width, height } = this.clip;
		this.crop =
			handle === 'move'
				? moveRect(start, dx, dy, width, height)
				: resizeRect(start, handle, dx, dy, width, height, aspectRatio(this.aspect));
	}

	setCropField(field: keyof CropRect, value: number) {
		this.crop = setField(this.crop, field, value, this.clip.width, this.clip.height, aspectRatio(this.aspect));
	}

	resetCrop() {
		this.crop = fullFrame(this.clip.width, this.clip.height);
		this.aspect = 'free';
	}

	normalizeCrop() {
		this.crop = snapRect(this.crop, this.clip.width, this.clip.height);
	}

	// ---------- Rename ----------

	/** The file moved on disk; keep the work and point the player at the new URL. */
	rebind(clip: Clip) {
		this.clip = clip;
	}

	// ---------- Export ----------

	/** This clip's edits, as one clip in an export of several. */
	buildItem(): ExportItem {
		return {
			path: this.clip.path,
			startSecs: round3(this.inSecs),
			endSecs: round3(this.outSecs),
			crop: this.cropActive ? { ...this.crop } : null,
			audioTracks: this.keptTracks,
			cuts: this.keptCuts.map((c) => ({ startSecs: round3(c.start), endSecs: round3(c.end) })),
			speeds: this.speeds.map((r) => ({ ...r })),
			camera:
				this.cameraPlacement && !this.cameraOff
					? {
							placement: { ...this.cameraPlacement },
							hidden: this.cameraHidden.map((r) => ({ startSecs: round3(r.start), endSecs: round3(r.end) }))
						}
					: null,
			logo: this.logo && !this.logoOff ? { path: this.logo.path, placement: { ...this.logo.placement } } : null,
			input: this.input && !this.inputOff ? { ...this.input, placement: { ...this.input.placement } } : null
		};
	}

	/** The export of this clip, followed by `more` clips. */
	buildRequest(more: ExportItem[] = []): ExportRequest {
		const item = this.buildItem();
		return {
			...item,
			overwrite: this.overwrite && more.length === 0 && this.exportOptions.format === 'mp4',
			name: this.overwrite ? null : this.exportName.trim() || null,
			target: exportTarget(this.exportOptions),
			more,
			format: this.exportOptions.format,
			fades: { ...this.fades, between: more.length > 0 ? this.fades.between : 0 },
			gifWidth: this.exportOptions.gifWidth,
			exactCuts: this.exportOptions.exactCuts
		};
	}

	async export(more: ExportItem[] = []): Promise<void> {
		if (!this.canExport && more.length === 0) return;
		this.playback.pause();
		this.exportState = 'running';
		this.exportFraction = 0;
		this.exportError = null;
		this.exportedClip = null;
		const request = this.buildRequest(more);
		let stop: (() => void) | null = null;
		beginExport(request.path, request.name || this.clip.fileName.replace(/\.mp4$/i, ''));
		try {
			stop = await api.onExportProgress((p) => {
				if (p.path !== request.path) return;
				this.exportFraction = Math.min(1, Math.max(0, p.fraction));
				updateExport(request.path, this.exportFraction);
			});
			const done: ExportResult = await api.exportClip(request);
			this.exportFraction = 1;
			this.exportState = 'done';
			this.exportedFile = done;
			const copied = this.exportOptions.copyAfter ? await copyToClipboard(done.path) : false;
			if (!done.clip) {
				// Another format: a file next to the clips, not a clip in the library.
				toast.success(copied ? 'Exported and copied' : 'Export finished', {
					description: `${done.fileName} · ${formatBytes(done.sizeBytes)}${copied ? ' · on the clipboard' : ''}`,
					action: { label: 'Show in folder', onClick: () => void revealClip(done.path) }
				});
				return;
			}
			const result = done.clip;
			this.exportedClip = result;
			upsertClip(result);
			if (copied) {
				toast.success('Exported and copied', {
					description: `${clipTitle(result)} is on the clipboard: paste it into Discord or a folder with Ctrl+V.`
				});
				if (request.overwrite) {
					dropSession(request.path);
					if (app.screen === 'editor') openClip(result.path);
				}
			} else if (request.overwrite) {
				dropSession(request.path);
				toast.success('Original replaced', {
					description: `${clipTitle(result)} · ${formatDuration(result.durationSecs)} · ${formatBytes(result.sizeBytes)}`
				});
				if (app.screen === 'editor') openClip(result.path);
			} else {
				toast.success('Export finished', {
					description: `${clipTitle(result)} · ${formatDuration(result.durationSecs)} · ${formatBytes(result.sizeBytes)}`,
					action: { label: 'Open', onClick: () => openClip(result.path) }
				});
			}
		} catch (error) {
			this.exportState = 'error';
			this.exportError = errorMessage(error);
			toast.error('Export failed', { description: this.exportError });
		} finally {
			stop?.();
			endExport(request.path);
		}
	}

	dismissExport() {
		if (this.exportState !== 'running') {
			this.exportState = 'idle';
			this.exportedClip = null;
			this.exportError = null;
		}
	}

	dispose() {
		this.playback.dispose();
	}
}

/** Puts a clip on the clipboard as a file. Says so when it fails; true when it worked. */
async function copyToClipboard(path: string): Promise<boolean> {
	try {
		await api.copyClipsToClipboard([path]);
		return true;
	} catch (error) {
		toast.error('Exported, but could not copy it', { description: errorMessage(error) });
		return false;
	}
}

// ---------- Session registry ----------

const sessions = new Map<string, EditSession>();

export function getSession(clip: Clip): EditSession {
	let session = sessions.get(clip.path);
	if (!session) {
		session = new EditSession(clip);
		sessions.set(clip.path, session);
	}
	return session;
}

export function dropSession(path: string) {
	sessions.get(path)?.dispose();
	sessions.delete(path);
}

/** After a rename the session follows the file to its new path. */
export function moveSession(oldPath: string, clip: Clip) {
	const session = sessions.get(oldPath);
	if (!session) return;
	sessions.delete(oldPath);
	session.rebind(clip);
	sessions.set(clip.path, session);
}

clipHooks.moved = moveSession;
clipHooks.removed = dropSession;
