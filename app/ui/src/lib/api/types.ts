// The contract between the UI and the Rust backend. The Rust side mirrors these types
// with serde (camelCase), so change both together.

// ---------- Settings ----------

export type VideoCodec = 'h264' | 'hevc' | 'av1';
export type RateMode = 'cbr' | 'vbr' | 'cq' | 'cqp';
/** `hq` high quality, `ll` low latency, `ull` ultra low latency. */
export type Tune = 'hq' | 'll' | 'ull';
/** A first look at each frame: off, at quarter resolution, or at full resolution. */
export type Multipass = 'off' | 'quarter' | 'full';
/** How a picture of another shape is fitted into the video: fill it, or keep the shape with black bars. */
export type Scaling = 'stretch' | 'fit';
export type RecordingMode = 'always' | 'games' | 'manual';
export type AudioMode = 'split' | 'system';

export type CaptureTarget =
	/** `device` is a name such as `\\.\DISPLAY1`. An empty string means the primary monitor. */
	| { kind: 'monitor'; device: string }
	/** A single window, matched by executable name first and title second. */
	| { kind: 'window'; exe: string; title: string };

export interface VideoSettings {
	codec: VideoCodec;
	/** Output size in pixels. `null` keeps the source size. Both are set or both are null. */
	width: number | null;
	height: number | null;
	/** Any whole number from 1 to 360. */
	fps: number;
	rateMode: RateMode;
	/** Target bitrate for `cbr` and `vbr`, in kbit/s. */
	bitrateKbps: number;
	/** Peak bitrate for `vbr`, in kbit/s. */
	maxBitrateKbps: number;
	/** Quantiser for `cqp`, 0 to 51. Lower is higher quality; about 10 is near-lossless. */
	qp: number;
	/** Quality for `cq`, 1 (best) to 51. The peak bitrate caps it. */
	cq: number;
	/** Encoder preset, 1 (fastest) to 7 (best quality). */
	preset: number;
	captureCursor: boolean;
	/** Seconds between keyframes, 1 to 10. A clip can only start on one. */
	keyframeSecs: number;
	tune: Tune;
	multipass: Multipass;
	/** Frames the encoder looks ahead, 0 to 32. */
	lookahead: number;
	spatialAq: boolean;
	temporalAq: boolean;
	/** 1 to 15. */
	aqStrength: number;
	scaling: Scaling;
	/** A screen in HDR: converted to SDR, or kept as HDR10 (HEVC and AV1 only). */
	hdr: 'sdr' | 'hdr10';
	/** Leave out the frames of a still screen; the encoder rests and the frame rate varies. */
	skipStill: boolean;
}

export interface AudioSettings {
	/** `split` records each source on its own track. `system` records everything on one track. */
	mode: AudioMode;
	/** Track for the game being recorded. Used in `split` mode. */
	game: boolean;
	/**
	 * Extra per-app tracks by executable name, for example `Discord.exe`. Used in `split` mode.
	 * New installs start with `['Discord.exe']`.
	 */
	apps: string[];
	/** Track for every other app. Used in `split` mode. */
	others: boolean;
	mic: {
		enabled: boolean;
		/** `null` means the default input device. */
		deviceId: string | null;
		/** When on, the microphone is only recorded while `pushToTalkKey` is held. */
		pushToTalk: boolean;
		/** Same format as `ClipSettings.hotkey`. A mouse button works too. */
		pushToTalkKey: string;
	/** Also mix the microphone into the combined track; it keeps its own track either way. */
	inMix: boolean;
	};
	/** Bitrate of each audio track, in kbit/s. */
	bitrateKbps: number;
}

export interface ClipSettings {
	/** How many seconds the replay buffer holds, and so how long a saved clip is. */
	lengthSecs: number;
	/**
	 * Modifiers then one key, joined by `+`. Modifiers are `Ctrl`, `Alt`, `Shift`, `Win`.
	 * The key is a `KeyboardEvent.code` value (`F10`, `KeyG`, `Digit1`, `Numpad5`) or a
	 * mouse button (`Mouse3` middle, `Mouse4` back, `Mouse5` forward). Example: `Alt+F10`.
	 */
	hotkey: string;
	saveSound: boolean;
	/** 0 to 1. */
	saveSoundVolume: number;
	/** Seconds still recorded after the hotkey before the clip is saved (0 to 15). */
	postSecs: number;
	/** Keep the replay buffer in files on disk instead of in memory. */
	bufferOnDisk: boolean;
	/** Folder for those files; empty uses Umbra's cache folder. */
	bufferFolder: string;
}

export type ScreenshotFormat = 'png' | 'jpg';

export interface ScreenshotSettings {
	/** Same format as `ClipSettings.hotkey`. */
	hotkey: string;
	/** `png` is lossless; `jpg` is smaller. */
	format: ScreenshotFormat;
	/** Plays the save sound (at the clip volume) when a screenshot is taken. */
	sound: boolean;
}

export interface RecordingSettings {
	mode: RecordingMode;
	launchAtStartup: boolean;
	startMinimized: boolean;
	/** Executable names the user marked as games, on top of automatic detection. */
	games: string[];
	/** What a game session records: the same as the replay buffer, or only the game's window. */
	sessionSource: SessionSource;
	/** Starts or stops a recording of what is being captured. */
	recordHotkey: string;
	/** Starts or stops recording the current game session. */
	sessionHotkey: string;
	/** Record every game session, from the game being found until it closes. */
	fullSessions: boolean;
	/** Pauses or resumes the recording to a file. */
	pauseHotkey: string;
	/** Programs that never count as games, so no session records them: names, paths or folders. */
	neverRecord: string[];
}

/** `manual`: started with the Record button or hotkey. `session`: a whole game session. */
export type RecordingKind = 'manual' | 'session';
export type SessionSource = 'capture' | 'gameWindow';
export type Corner = 'topLeft' | 'topRight' | 'bottomLeft' | 'bottomRight';

/** Video settings for a kind of game, used while one of its games is recorded. */
export interface VideoPreset {
	id: string;
	name: string;
	/** Program names, such as `cs2.exe`. */
	games: string[];
	video: VideoSettings;
}

export interface ExportSettings {
	/** Processor threads for decoding, 0 for one per core. */
	threads: number;
	/** Burn in (from the library) puts the new file in the original's place once it checks out. */
	burnReplaceOriginal: boolean;
}

export type OverlayShape = 'rectangle' | 'rounded' | 'circle';

/** A rectangle as fractions (0 to 1) of a picture. */
export interface FracRect {
	x: number;
	y: number;
	width: number;
	height: number;
}

/**
 * Where a picture (the camera, the logo) sits over the video and how it looks. Positions are
 * fractions of the video, so it fits any size; the height follows the shape of what is shown.
 */
export interface Placement {
	x: number;
	y: number;
	width: number;
	/** The part of the picture shown. */
	crop: FracRect;
	shape: OverlayShape;
	/** Border thickness as a fraction of the shown width; 0 for none. */
	border: number;
	/** CSS colour, such as `#ffffff`. */
	borderColor: string;
	/** Flipped left to right. */
	mirror: boolean;
}

/** A webcam recorded as a stream of its own and drawn over the video by the player and export. */
export interface CameraSettings {
	/** Filming; off leaves the recorder running, with the camera's light out. */
	enabled: boolean;
	/** Windows' id of the camera; null records no camera. */
	deviceId: string | null;
	/** Its name, shown while it is not connected. */
	deviceName: string;
	/** Size it is recorded at: one of the camera's own sizes. */
	width: number;
	height: number;
	/** Where new clips show it. */
	placement: Placement;
	/** Switches the camera on and off. */
	hotkey: string;
}

/** A picture shown over every new clip; movable or removable in the editor. Not recorded. */
export interface LogoSettings {
	/** A copy of the chosen image in Umbra's folder; null for none. */
	path: string | null;
	placement: Placement;
}

/** Keyboard and mouse input recorded with clips, to show over them. Off unless switched on. */
export interface InputSettings {
	enabled: boolean;
	/** Switches recording input on and off; empty for none. */
	hotkey: string;
	/** What is kept in which program; the first that matches applies. */
	rules: InputRule[];
	/** How the overlay looks on new clips, until moved in the editor. */
	overlay: InputOverlay;
}

/** `redacted` keeps keys that type text without saying which. */
export type KeyboardMode = 'full' | 'redacted' | 'drop';
/** `buttonsOnly` keeps buttons and the wheel, not where the mouse goes. */
export type MouseMode = 'full' | 'buttonsOnly' | 'drop';

export interface InputRule {
	name: string;
	/** `game` is the game Umbra detected; `apps` the programs in `apps`; `any` every program. */
	matches: 'game' | 'apps' | 'any';
	/** Program names, paths or folders, as for video presets. */
	apps: string[];
	keyboard: KeyboardMode;
	mouse: MouseMode;
	/** Keys that open a chat (`Enter`, `KeyY`); typed text is then redacted until Enter or Esc. */
	chatKeys: string[];
}

export interface InputOverlay {
	keys: 'none' | 'wasd' | 'keyboard';
	mouse: boolean;
	/** The mouse shows which way it moves. */
	movement: boolean;
	/** A line of the keys pressed last. */
	ticker: boolean;
	/** Where, and how wide; the height follows the layout. Only x, y and width are used. */
	placement: Placement;
	/** 0.1 to 1. */
	opacity: number;
	/** CSS colour of keys and buttons held down. */
	highlight: string;
}

/** What the input capture is doing right now. */
export interface InputStatus {
	on: boolean;
	/** The program in front, such as `chrome.exe`. */
	app: string | null;
	/** The rule that applies to it; null when nothing is kept there. */
	rule: string | null;
	/** While a password field has focus, or a game's chat is open. */
	guard: 'password' | 'chat' | null;
}

/**
 * A clip's input (`getInputTrack`): blocks of what was held at their start and what happened
 * after, in seconds of the clip. Events are `[seconds, kind, a, b, c]`: 1 key down and 2 up
 * (`a` virtual key, `b` scan code, `c` 1 for an extended key), 3 and 4 a text key down and up,
 * 5 and 6 a mouse button down and up (`a`: 0 left, 1 right, 2 middle, 3 back, 4 forward),
 * 7 the wheel (`a` up, `b` right), 8 movement (`a` right, `b` down).
 */
export interface InputTrack {
	blocks: InputBlock[];
}

export interface InputBlock {
	start: number;
	/** Keys held: [virtual key, scan code, extended]. */
	keys: [number, number, number][];
	/** Text keys held. */
	text: number;
	/** One bit per mouse button. */
	buttons: number;
	events: [number, number, number, number, number][];
}

/** Clips saved by themselves when something happens in a game. */
export interface AutoClipSettings {
	enabled: boolean;
	/** In games without rounds, events this close to the one before go in the same clip (0 to 60 s). */
	mergeSecs: number;
	/** Volume of the sound for an auto-clip, 0 to 1; 0 is silent. */
	soundVolume: number;
	/** Kills a clip needs, 1 to 5 (5 is an ace). Counted per round where the game has rounds. */
	minKills: number;
	kill: EventRule;
	/** Special kills: always in the clip's name; switched on, one makes a clip by itself. */
	headshot: EventRule;
	knife: EventRule;
	zeus: EventRule;
	collateral: EventRule;
	blindKill: EventRule;
	death: EventRule;
	roundWin: EventRule;
	cs2: Cs2Settings;
}

export interface EventRule {
	/** Makes a clip. Off, the event is only named in a clip something else made. */
	enabled: boolean;
	/** Events a detector is less sure of (0 to 1) are left out. */
	minConfidence: number;
	/** Seconds kept before (0 to 120) and after (0 to 60) the event. */
	preSecs: number;
	postSecs: number;
}

/** Counter-Strike 2's Game State Integration: the game posts its state to 127.0.0.1. */
export interface Cs2Settings {
	enabled: boolean;
	port: number;
	/** Made by the backend; the game sends it back with every post. */
	token: string;
	/** Game modes clipped: competitive, wingman, casual, deathmatch, armsrace, other. */
	modes: string[];
}

export type GameEventKind =
	| 'kill'
	| 'headshot'
	| 'knife'
	| 'zeus'
	| 'collateral'
	| 'blindKill'
	| 'death'
	| 'roundWin'
	| 'roundDone';

/** One line of the auto-clip log: an event that came in, or what was done about it. */
export interface AutoClipLogEntry {
	unixMs: number;
	kind: GameEventKind | null;
	confidence: number | null;
	source: string | null;
	message: string;
}

/** CS2's config file, for copying by hand; `folder` is where it goes, when CS2 is found. */
export interface Cs2Config {
	text: string;
	fileName: string;
	folder: string | null;
}

export interface CameraInfo {
	id: string;
	name: string;
	/** The sizes it films at, smallest first; none for a software camera that films nothing now. */
	sizes: { width: number; height: number }[];
	/** How Umbra reads it: `Media Foundation` (webcams) or `DirectShow` (software cameras such as OBS's). */
	api: string;
}

/** A clip's camera as a file of its own, to show over the clip. */
export interface CameraVideo {
	url: string;
	/** Where the camera was filming, as [start, end] in clip seconds. The file's time 0 is the
	 * first one's start. */
	stretches: [number, number][];
}

export interface OverlaySettings {
	/** Short messages over the game when a clip is saved or a recording starts. */
	enabled: boolean;
	corner: Corner;
	/** A badge that stays on screen (recording, replay buffer): never, always, or with games. */
	status: 'off' | 'always' | 'games';
	/** Programs it shows for, by name, path or folder; empty is any game. */
	statusGames: string[];
	statusCorner: Corner;
}

export interface LoadSuggestion {
	text: string;
	/** The video settings with the change made, if it is one. */
	video: VideoSettings | null;
	/** A new clip length in seconds, if that is the change. */
	clipSecs: number | null;
}

export interface LoadReport {
	alone: { fps: number; lowFps: number };
	recording: { fps: number; lowFps: number };
	/** Share of the game's frame rate (and 1 % lows) recording took, in percent. */
	fpsCost: number;
	lowCost: number;
	framesRecorded: number;
	framesDropped: number;
	dropPercent: number;
	gameMemory: number;
	bufferMemory: number;
	totalMemory: number;
	spec: { width: number; height: number; targetFps: number; cpuThreads: number };
	suggestions: LoadSuggestion[];
}

export interface LoadProgress {
	label: string;
	/** 0 to 1. */
	fraction: number;
}

export interface StorageUsage {
	bytes: number;
	files: number;
	/** Free on the clip folder's drive. */
	freeBytes: number;
}

/** Another recorder's folder on this PC with videos in it. */
export interface ImportSource {
	label: string;
	path: string;
	files: number;
	bytes: number;
}

/** A video that importing would bring in. */
export interface ImportFile {
	path: string;
	name: string;
	bytes: number;
	/** From its folder or file name; it goes into a folder of this name. */
	game: string;
	/** Already in the library under that game; left out. */
	duplicate: boolean;
}

/** Files dragged over the window: their paths on enter and drop. */
export type FileDrop =
	| { type: 'enter'; paths: string[] }
	| { type: 'over' }
	| { type: 'drop'; paths: string[] }
	| { type: 'leave' };

/** A file being imported right now. */
export interface ImportActive {
	name: string;
	bytes: number;
	/** Copying (or rewrapping), converting (re-encoding), checking the copy, removing the original. */
	stage: 'copying' | 'converting' | 'checking' | 'removing';
	/** Of this stage, 0 to 1. */
	fraction: number;
}

export interface ImportProgress {
	/** Files finished, however they ended. */
	done: number;
	total: number;
	/** The last file that finished. */
	current: string;
	/** Work done and to do, in bytes read: copying, and checking the copy when moving. */
	bytesDone: number;
	bytesTotal: number;
	/** Bytes per second over the last few seconds. */
	rate: number;
	/** Seconds left at that rate, once it is known. */
	etaSecs: number | null;
	active: ImportActive[];
	/** Originals are removed once their copy checks out. */
	moving: boolean;
}

export interface ImportDone {
	imported: number;
	skipped: number;
	/** Originals removed after their copy was checked (moving). */
	removed: number;
	/** "name: why" for each file that could not be imported. */
	failed: string[];
}

/** The clip folder's drive is nearly full: a recording was stopped, or could not start. */
export interface LowDisk {
	title: string;
	detail: string;
}

/** What a storage cleanup removed. */
export interface Cleanup {
	files: number;
	bytes: number;
}

export interface RecordingFinishing {
	kind: RecordingKind;
	game: string;
}

export type ShrinkQuality = 'small' | 'balanced' | 'high';

/** Layers of a clip drawn over it in Umbra: those it has, or those to burn in. */
export interface BurnLayers {
	camera: boolean;
	input: boolean;
	logo: boolean;
}

export interface BurnResult {
	/** The clip burned into. */
	path: string;
	fileName: string;
	/** The new file when it worked: beside the clip, or the clip's own path when it replaced it. */
	output: string | null;
	problem: string | null;
	/** You stopped it. */
	stopped: boolean;
}

export interface ShrinkResult {
	path: string;
	fileName: string;
	savedBytes: number | null;
	problem: string | null;
	/** You stopped it. */
	stopped: boolean;
}

/** A game running now, to pick from a list. */
export interface RunningGame {
	exe: string;
	name: string;
}

/** What shrinking old recordings would do now. */
export interface ShrinkEstimate {
	files: number;
	bytes: number;
	afterBytes: number;
	running: boolean;
}

/** A file Umbra is still working on, such as a long recording being made quick to open. */
export interface ProcessingItem {
	/** `finish` (made quick to open) or `shrink` (made smaller). */
	task: 'finish' | 'shrink' | 'burn';
	/** Where the file ends up. */
	path: string;
	fileName: string;
	game: string;
	kind: RecordingKind;
	bytesDone: number;
	bytesTotal: number;
	startedAt: string;
	/** Seconds left at the speed so far, once there is one. */
	etaSecs: number | null;
}

/** A recording to a file in progress; it runs next to the replay buffer. */
export interface FileRecordingStatus {
	kind: RecordingKind;
	/** Seconds recorded so far. */
	secs: number;
	bytes: number;
	/** The game or program, or `Desktop`. */
	game: string;
	path: string;
	/** What it records, for a live preview. */
	source: CaptureTarget;
	/** Nothing is added until it is resumed; the file has no gap. */
	paused: boolean;
}

export interface RecordingSaved {
	/** The finished file, or null if nothing could be kept. */
	clip: Clip | null;
	kind: RecordingKind;
	game: string;
	/** Why it ended early, or a problem finishing it. */
	error: string | null;
}

export interface StorageSettings {
	/** Base folder for clips. */
	folder: string;
	/** Subfolder template under `folder`. May be empty. Example: `{game}`. */
	folderTemplate: string;
	/** File name template without extension. Example: `{game} {date} {time}`. */
	nameTemplate: string;
	/** Delete files for good instead of moving them to the Recycle Bin. */
	skipRecycleBin: boolean;
	/** Keep the clip folder under this many gigabytes by deleting the oldest clips; 0 is off. */
	limitGb: number;
	/** Delete clips older than this many days; 0 is off. */
	maxAgeDays: number;
	/** Never delete clips in an album. Favourites are always kept. */
	keepAlbums: boolean;
	/** Stop recordings when the clips drive has less free than this many GB; 0 is off. */
	minFreeGb: number;
	/** Make old recordings smaller in the background. */
	shrink: {
		enabled: boolean;
		afterDays: number;
		quality: 'small' | 'balanced' | 'high';
		includeClips: boolean;
		/** At most this many frames per second afterwards; 0 keeps the recording's. */
		maxFps: number;
	};
}

/** Placeholders allowed in the storage templates. */
export const TEMPLATE_PLACEHOLDERS = [
	{ token: '{game}', meaning: 'Game or app name, or "Desktop"' },
	{ token: '{date}', meaning: 'Date as 2026-09-30' },
	{ token: '{time}', meaning: 'Time as 18-05-42' },
	{ token: '{year}', meaning: 'Year as 2026' },
	{ token: '{month}', meaning: 'Month as 09' },
	{ token: '{day}', meaning: 'Day as 30' }
] as const;

/** How the clip player and the editor move through a clip. */
export interface PlayerSettings {
	/** Seconds the arrow keys skip, 0.1 to 600. */
	seekSecs: number;
}

export interface Settings {
	capture: CaptureTarget;
	video: VideoSettings;
	/** Video settings for kinds of games; `video` covers everything else. */
	presets: VideoPreset[];
	audio: AudioSettings;
	clip: ClipSettings;
	screenshot: ScreenshotSettings;
	recording: RecordingSettings;
	storage: StorageSettings;
	player: PlayerSettings;
	export: ExportSettings;
	overlay: OverlaySettings;
	/** The recording panel's widgets, top to bottom (see `lib/panel/widgets.ts`). */
	panel: { widgets: string[] };
	camera: CameraSettings;
	logo: LogoSettings;
	autoClip: AutoClipSettings;
	input: InputSettings;
}

// ---------- Live state ----------

export interface Status {
	/** `recording` means the replay buffer is running. */
	state: 'idle' | 'recording' | 'error';
	/** Set when `state` is `error`. */
	error: string | null;
	/** What is being captured, for example `DISPLAY2, 2560x1440`. */
	target: string | null;
	/** Detected game, if any. */
	game: string | null;
	/** Why the game counts as one, for example `installed through Steam`. */
	gameReason: string | null;
	/** Encoder in use, for example `hevc_nvenc`. */
	encoder: string | null;
	/** The screen is in HDR: `sdr` when converted, `hdr10` when kept; null otherwise. */
	hdr?: string | null;
	width: number;
	height: number;
	fps: number;
	/** Seconds of footage currently in the buffer. */
	bufferSecs: number;
	/** RAM used by the buffer, in bytes. */
	bufferBytes: number;
	/** The buffer is on disk: `bufferBytes` is disk space, not memory. */
	bufferOnDisk: boolean;
	/** The microphone is being recorded right now (with push-to-talk: only while the key is held). */
	micLive: boolean;
	/** Push-to-talk is on, so `micLive` follows the key. */
	pushToTalk: boolean;
	/** Frames the encoder could not keep up with since recording started. */
	droppedFrames: number;
	/** What Umbra itself uses right now. Sent at least every 2 seconds. */
	usage: Usage;
	/** The recording to a file, while one runs. */
	file: FileRecordingStatus | null;
	/** The session of the running game was stopped by hand; it can be restarted. */
	sessionStopped: boolean;
	/** Audio tracks recorded as silence because their source cannot be opened (a microphone
	 * that is not connected). The microphone's track is called `Microphone`. */
	audioProblems: AudioProblem[];
	/** The camera is switched on (the setting; the hotkey and the tray change it too). */
	cameraOn: boolean;
	/** The camera is being recorded right now. */
	cameraLive: boolean;
	/** Why the camera is not recorded while it should be, such as it not being connected. */
	cameraProblem: string | null;
	input: InputStatus;
}

export interface AudioProblem {
	track: string;
	problem: string;
}

export interface Usage {
	/** Share of the whole CPU used by Umbra, 0 to 100. */
	cpuPercent: number;
	/** Memory used by Umbra's process, replay buffer included, in bytes. */
	memoryBytes: number;
	/** Share of the GPU's 3D and compute engines used by Umbra, 0 to 100. Null if Windows doesn't say. */
	gpuPercent: number | null;
	/** Share of the GPU's video encoder used by Umbra, 0 to 100. Null if Windows doesn't say. */
	encoderPercent: number | null;
}

/** What the current (or proposed) settings will cost, worked out before recording. */
export interface UsageEstimate {
	/** RAM for the replay buffer at its fullest, in bytes. */
	bufferBytes: number;
	/** Size of one saved clip, in bytes. */
	clipBytes: number;
	/** How busy the GPU's video encoder will be, 0 to 1. Above 1 means it cannot keep up. */
	encoderLoad: number;
	/** Expected share of the whole CPU, 0 to 100. */
	cpuPercent: number;
	/** Overall verdict. */
	level: 'light' | 'moderate' | 'heavy' | 'too-heavy';
	/** Short plain-language notes, for example why the level is heavy. */
	notes: string[];
	/** What each costly setting costs, most costly first. */
	breakdown: Cost[];
}

/** One setting's cost: what it uses, why, what to change and what that gives up. */
export interface Cost {
	resource: 'encoder' | 'gpu' | 'cpu' | 'memory' | 'disk';
	impact: 'high' | 'medium' | 'low';
	title: string;
	why: string;
	change: string;
	catch: string;
}

/** Something that can be recorded, for the quick switcher. */
export interface CaptureSource {
	target: CaptureTarget;
	/** For example `Display 1` or the window title. */
	label: string;
	/** For example `2560×1440 · 360 Hz` or `eldenring.exe`. */
	detail: string;
	/** A window whose program looks like a game (listed in `recording.games` or fills its screen). */
	isGame: boolean;
}

export interface MonitorInfo {
	device: string;
	width: number;
	height: number;
	refreshHz: number;
	primary: boolean;
	/** The monitor's own resolution; differs from width × height while a game runs the screen stretched. */
	nativeWidth: number;
	nativeHeight: number;
}

export type GameKind = 'shooter' | 'story' | 'mixed';
/** How much game performance recording may take. */
export type BenchmarkCost = 'least' | 'some' | 'most';

export interface BenchmarkAnswers {
	game: GameKind;
	cost: BenchmarkCost;
	/** A fixed recording size, or null for the screen's own. */
	size: [number, number] | null;
	/** A fixed frame rate, or null to let the benchmark choose. */
	fps: number | null;
	scaling: Scaling;
	/** Clips mostly go to Discord or the web. */
	share: boolean;
}

export interface BenchmarkTrial {
	codec: VideoCodec;
	width: number;
	height: number;
	fps: number;
	preset: number;
	/** Frames per second the encoder managed flat out; null if it would not start. */
	measuredFps: number | null;
	/** Share of the encoder that recording at `fps` uses. */
	load: number | null;
	passed: boolean;
	error: string | null;
}

export interface BenchmarkReport {
	/** The current video settings with the recommendation applied. */
	video: VideoSettings;
	trials: BenchmarkTrial[];
	notes: string[];
	/** Nothing passed; this is the fastest setting there is. */
	compromise: boolean;
}

export interface BenchmarkProgress {
	label: string;
	trial: BenchmarkTrial | null;
}

export interface WindowInfo {
	exe: string;
	title: string;
}

export interface AudioAppInfo {
	exe: string;
	/** Display name, for example `Discord`. */
	name: string;
}

export interface MicrophoneInfo {
	id: string;
	name: string;
	isDefault: boolean;
}

export interface SystemInfo {
	appVersion: string;
	gpu: string;
	/** Codecs the hardware encoder on this machine supports. */
	codecs: VideoCodec[];
	/** Folder holding `settings.json`, albums and the thumbnail cache. */
	settingsFolder: string;
}

// ---------- Clips ----------

/**
 * How a track relates to the combined first track (`Mix`) that most players play:
 * - `mix`: the combined track. On export it is rebuilt from the `inMix` tracks at their
 *   volumes when any of them changes.
 * - `inMix`: summed into it.
 * - `inOthers`: an app with its own track that older recordings also have inside
 *   `Other apps` (and so in `Mix` through it).
 * - `separate`: only on its own track, like the microphone, or any track of a clip without
 *   a combined track.
 */
export type MixRole = 'mix' | 'inMix' | 'inOthers' | 'separate';

export interface ClipAudioTrack {
	index: number;
	/** For example `Game`, `Discord`, `Other apps`, `Microphone`. */
	title: string;
	role: MixRole;
}

export interface Clip {
	/** Absolute path; also the clip's identity. */
	path: string;
	/** `image` is a screenshot: no duration, no audio, opens in a viewer instead of the editor. */
	kind: 'video' | 'image';
	fileName: string;
	game: string | null;
	/** ISO 8601. */
	createdAt: string;
	durationSecs: number;
	sizeBytes: number;
	width: number;
	height: number;
	fps: number;
	codec: VideoCodec;
	/** URL usable in an `<img>`, or null while the thumbnail is being made. */
	thumbnailUrl: string | null;
	/** URL usable in a `<video>`, or in an `<img>` when `kind` is `image`. */
	videoUrl: string;
	audioTracks: ClipAudioTrack[];
	/** Ids of the albums this clip is in. */
	albums: string[];
	/** Average bitrate of the whole file in bit/s; 0 if unknown. */
	bitRate: number;
	/**
	 * How the clip was recorded, as Umbra wrote it into the file. Keys: `game`, `game_exe`,
	 * `window`, `detected` (captured window, listed game, fullscreen program, no game),
	 * `source`, `encoder`, `gpu`, `rate`, `preset`, `recorded_fps`, `app`. Empty for clips from
	 * before Umbra stored this, and for files from elsewhere.
	 */
	details: Record<string, string>;
	/** For a recording: seconds where the clip hotkey was pressed while it recorded. */
	bookmarks: number[];
	/** Size of the webcam recorded next to the screen; null without one. Where it shows is
	 * in `details.camera` (a `Placement` as JSON), the logo in `details.logo`. */
	camera: { width: number; height: number } | null;
	/** Keyboard and mouse input was recorded with it. How its overlay looks is in
	 * `details.input` (an `InputOverlay` as JSON). */
	input: boolean;
	/** A logo shows over it (`details.logo`), and its picture is still there. */
	logo: boolean;
}

/** The id of the built-in Favorites album. */
export const FAVORITES = 'favorites';

export interface Album {
	id: string;
	name: string;
	/** Clips in this album are not shown on the Clips screen. */
	hideFromClips: boolean;
	/** Paths of the clips in the album, oldest addition first. Missing files are dropped. */
	clipPaths: string[];
	/** ISO 8601. */
	createdAt: string;
	/** Favorites: always there, first, and cannot be renamed or deleted. */
	builtin: boolean;
}

/** One audio track of a clip as its own file, for previewing tracks at different volumes. */
export interface TrackAudio {
	index: number;
	/** URL usable in an `<audio>` element or with Web Audio. */
	url: string;
	/**
	 * `track`: a track as recorded. `othersWithoutApps`: "Other apps" (whose index this has)
	 * with the apps listed in `apps` taken out, so each app can be heard at its own volume.
	 */
	kind: 'track' | 'othersWithoutApps';
	apps: number[];
}

export interface CropRect {
	x: number;
	y: number;
	width: number;
	height: number;
}

export interface ExportRequest {
	path: string;
	startSecs: number;
	endSecs: number;
	/** In source pixels. `null` keeps the full frame. */
	crop: CropRect | null;
	/**
	 * Audio tracks to keep, with a volume for each: 1 leaves it as recorded, 0.5 halves it,
	 * up to 4. Tracks not listed are left out. Any volume other than 1 re-encodes that track.
	 */
	audioTracks: { index: number; volume: number }[];
	/**
	 * Ranges to leave out, in seconds from the start of the clip; the parts either side are
	 * joined. Any range re-encodes the video.
	 */
	cuts: CutRange[];
	/** Replace the original instead of writing a new file next to it. */
	overwrite: boolean;
	/** The new file's name without .mp4; null uses the clip's name with "(edit)". */
	name?: string | null;
	/** What a preset asks of the file; null exports as the original is. */
	target?: ExportTarget | null;
	/** Stretches of this clip at another speed, in its own seconds. */
	speeds?: SpeedRange[];
	/** More clips after this one, each with its own edits. */
	more?: ExportItem[];
	format?: OutputFormat;
	fades?: Fades;
	/** Width of a GIF; its height follows. */
	gifWidth?: number | null;
	/** Cut on the exact frame by re-encoding; otherwise cuts move to the nearest keyframe and the packets are copied. */
	exactCuts?: boolean;
	/** The camera drawn in where the editor placed it, hidden for `hidden`; null leaves it out. */
	camera?: { placement: Placement; hidden: CutRange[] } | null;
	/** A picture drawn in; null leaves it out. */
	logo?: { path: string; placement: Placement } | null;
	/** The clip's keyboard and mouse input drawn in; null leaves it out. */
	input?: InputOverlay | null;
}

/** A stretch played at another speed: 0.5 is half speed, 2 twice as fast. */
export interface SpeedRange {
	start: number;
	end: number;
	speed: number;
}

/** In seconds: from black at the start, to black at the end, and through black between clips. */
export interface Fades {
	fadeIn: number;
	fadeOut: number;
	between: number;
}

export type OutputFormat = 'mp4' | 'mov' | 'mkv' | 'webm' | 'gif' | 'mp3' | 'm4a' | 'wav' | 'flac' | 'opus';

/** A further clip in an export of several, with its own edits. */
export interface ExportItem {
	path: string;
	startSecs: number;
	endSecs: number;
	crop: CropRect | null;
	audioTracks: { index: number; volume: number }[];
	cuts: CutRange[];
	speeds: SpeedRange[];
	camera?: ExportRequest['camera'];
	logo?: ExportRequest['logo'];
	input?: ExportRequest['input'];
}

/** What an export made: the new clip when it is an MP4 in the library, otherwise only the file. */
export interface ExportResult {
	clip: Clip | null;
	path: string;
	fileName: string;
	sizeBytes: number;
}

/** What an export preset asks of the file. */
export interface ExportTarget {
	codec?: VideoCodec;
	/** The file may be at most this big; the bitrate and picture size follow from it. */
	maxBytes?: number;
	/** Video bitrate in kbit/s when no size limit sets it. */
	videoKbps?: number;
	maxHeight?: number;
	maxFps?: number;
	/** Only the combined track, with the microphone mixed in. */
	mixOnly?: boolean;
}

export interface CutRange {
	startSecs: number;
	endSecs: number;
}

/** Loudness outlines of a clip's audio tracks, for drawing waveforms. */
export interface Waveforms {
	/** Length of one slice in seconds; slice `i` starts at `i * sliceSecs`. */
	sliceSecs: number;
	/** Per audio track (file order), one level per slice: 0 is -60 dB or quieter, 255 full scale. */
	tracks: number[][];
}

export interface ExportProgress {
	path: string;
	/** 0 to 1. */
	fraction: number;
}

// ---------- Backend surface ----------

export interface Api {
	getSettings(): Promise<Settings>;
	/** Returns the settings as stored, after validation. */
	setSettings(settings: Settings): Promise<Settings>;
	getStatus(): Promise<Status>;
	getSystemInfo(): Promise<SystemInfo>;

	startRecording(): Promise<void>;
	stopRecording(): Promise<void>;
	saveClip(): Promise<Clip>;
	/** Captures the current target as an image and saves it next to the clips. */
	takeScreenshot(): Promise<Clip>;

	listMonitors(): Promise<MonitorInfo[]>;
	listWindows(): Promise<WindowInfo[]>;
	/** Games running now, in front or not. */
	listRunningGames(): Promise<RunningGame[]>;
	listAudioApps(): Promise<AudioAppInfo[]>;
	listMicrophones(): Promise<MicrophoneInfo[]>;
	/** Monitors, then windows, for the quick switcher. */
	listCaptureSources(): Promise<CaptureSource[]>;
	/** A small still of a source as a `data:image/jpeg` URL, or null if it can't be captured (minimised window). */
	getPreview(target: CaptureTarget, maxWidth: number): Promise<string | null>;
	/** Works out what `settings` would cost without applying them. */
	estimateUsage(settings: Settings): Promise<UsageEstimate>;
	/** The settings a new install starts with. */
	getDefaultSettings(): Promise<Settings>;
	/** How much the clip folder holds. */
	getStorageUsage(): Promise<StorageUsage>;
	/** Applies the storage limits now. */
	cleanUpStorage(): Promise<Cleanup>;
	/** True on a PC Umbra has not been set up on, until `finishOnboarding`. */
	isFirstRun(): Promise<boolean>;
	/** Stores the settings and ends the first-start screen. */
	finishOnboarding(): Promise<void>;
	/** Starts recording what is being captured to a file; the replay buffer keeps running. */
	/** Records the capture, or `source` (a display or a window) next to the replay buffer. */
	startFileRecording(kind: RecordingKind, source?: CaptureTarget | null): Promise<void>;
	/** Stops the recording and keeps it; `onRecordingSaved` follows. */
	stopFileRecording(): Promise<void>;
	/** Pauses (`true`) or resumes (`false`) the recording to a file. */
	pauseFileRecording(paused: boolean): Promise<void>;
	/** Measures the encoder and recommends video settings, pausing recording meanwhile. Applies nothing. */
	runBenchmark(answers: BenchmarkAnswers): Promise<BenchmarkReport>;
	/** Records under a stand-in game's load with the default settings or a preset's. Stopped with `cancelBenchmark`. */
	runLoadTest(presetId: string | null): Promise<LoadReport>;
	cancelBenchmark(): Promise<void>;
	/** A short code that holds every video setting, to share. */
	videoSettingsCode(video: VideoSettings): Promise<string>;
	/** Video settings from a share code. Rejects with a readable message for a bad code. */
	readVideoSettingsCode(code: string): Promise<VideoSettings>;
	/** Opens the settings folder in Explorer. */
	openSettingsFolder(): Promise<void>;

	listClips(): Promise<Clip[]>;
	/** Moves the file to the recycle bin. */
	deleteClip(path: string): Promise<void>;
	/** Moves several files to the recycle bin in one go. */
	deleteClips(paths: string[]): Promise<void>;
	renameClip(path: string, newName: string): Promise<Clip>;
	revealClip(path: string): Promise<void>;
	exportClip(request: ExportRequest): Promise<ExportResult>;
	/** Hands the files to Windows' drag and drop, to drop into another program. True when dropped. */
	dragClipsOut(paths: string[]): Promise<boolean>;
	/** Puts clips on the clipboard as files, ready to paste into Discord or a folder. A clip with
	 * a camera or logo is exported with them drawn in first, so this can take a while. */
	copyClipsToClipboard(paths: string[]): Promise<void>;
	/** Splits a clip's audio into one file per track (cached), for the editor's per-track preview. */
	getTrackAudio(path: string): Promise<TrackAudio[]>;
	/** A clip's camera as a file of its own (cached); null for a clip without one. */
	getCameraVideo(path: string): Promise<CameraVideo | null>;
	/** Waveform outlines of every audio track. Decodes the audio, so it takes a moment. */
	getWaveforms(path: string): Promise<Waveforms>;

	listAlbums(): Promise<Album[]>;
	createAlbum(name: string): Promise<Album>;
	renameAlbum(id: string, name: string): Promise<Album>;
	/** Deletes the album only; its clips stay where they are. */
	deleteAlbum(id: string): Promise<void>;
	setAlbumHidden(id: string, hideFromClips: boolean): Promise<Album>;
	addToAlbum(id: string, paths: string[]): Promise<Album>;
	removeFromAlbum(id: string, paths: string[]): Promise<Album>;

	/** Opens the system folder picker. Resolves to null if the user cancels. */
	pickFolder(): Promise<string | null>;
	/** The system file picker for a program (.exe); null if cancelled. */
	pickProgram(): Promise<string | null>;
	/** The system file picker for a logo image, copied into Umbra's folder; its copy's path, or null if cancelled. */
	pickLogo(): Promise<string | null>;
	/** Webcams connected now, with the sizes they film at. */
	listCameras(): Promise<CameraInfo[]>;
	/** A URL for a local image (a logo), usable in an `<img>`. */
	fileUrl(path: string): Promise<string>;

	onStatus(handler: (status: Status) => void): Promise<() => void>;
	onClipSaved(handler: (clip: Clip) => void): Promise<() => void>;
	/** A clip is being saved (the hotkey or Save clip): after `waitSecs`, the last `lengthSecs`. */
	onClipSaving(handler: (event: { game: string; waitSecs: number; lengthSecs: number }) => void): Promise<() => void>;
	onClipFailed(handler: (event: { error: string }) => void): Promise<() => void>;
	onExportProgress(handler: (progress: ExportProgress) => void): Promise<() => void>;
	/**
	 * Fires (no payload) when the clip list may have changed: after thumbnails finish generating
	 * in the background, and after a save, delete, rename or export. Re-fetch with `listClips()`.
	 */
	onClipsChanged(handler: () => void): Promise<() => void>;
	/** Fires (no payload) after any album change. Re-fetch with `listAlbums()`. */
	onAlbumsChanged(handler: () => void): Promise<() => void>;
	onBenchmarkProgress(handler: (progress: BenchmarkProgress) => void): Promise<() => void>;
	onRecordingSaved(handler: (saved: RecordingSaved) => void): Promise<() => void>;
	onRecordingFinishing(handler: (event: RecordingFinishing) => void): Promise<() => void>;
	/** Files being worked on (a recording being rewritten), for the library to show. */
	listProcessing(): Promise<ProcessingItem[]>;
	shrinkEstimate(): Promise<ShrinkEstimate>;
	/** Compresses these clips now, at `quality` or the setting's. */
	compressClips(paths: string[], quality?: ShrinkQuality | null): Promise<{ queued: number; waiting: boolean }>;
	/** A clip asked for by hand was compressed, or left as it was and why. */
	onShrinkResult(handler: (result: ShrinkResult) => void): Promise<() => void>;
	/** Stops compressing these clips, the one being compressed and any waiting; returns how many. */
	stopCompressing(paths: string[]): Promise<number>;
	/** Queues clips to burn these layers into, one after another; returns how many were queued. */
	burnIn(paths: string[], layers: BurnLayers): Promise<number>;
	/** Stops burning into these clips (or into the new files), the one at work and any waiting. */
	stopBurnIn(paths: string[]): Promise<number>;
	/** A clip was burned into, or not and why. */
	onBurnResult(handler: (result: BurnResult) => void): Promise<() => void>;
	shrinkNow(): Promise<void>;
	onProcessingStarted(handler: () => void): Promise<() => void>;
	onStorageCleaned(handler: (cleanup: Cleanup) => void): Promise<() => void>;
	onLowDisk(handler: (event: LowDisk) => void): Promise<() => void>;
	/** A clip's keyboard and mouse input; no blocks when it has none. */
	getInputTrack(path: string): Promise<InputTrack>;
	/** Switches recording input on or off (`null` switches it over), and remembers it. */
	setInputCapture(on: boolean | null): Promise<void>;
	/** The auto-clip log so far, oldest first. */
	getAutoClipLog(): Promise<AutoClipLogEntry[]>;
	onAutoClipLog(handler: (entry: AutoClipLogEntry) => void): Promise<() => void>;
	/** Writes CS2's config file into the game's cfg folder (applied port and token); returns its path. */
	installCs2Config(): Promise<string>;
	getCs2Config(): Promise<Cs2Config>;
	/** Videos picked with the system file picker; empty if cancelled. */
	pickVideos(): Promise<string[]>;
	listImportSources(): Promise<ImportSource[]>;
	scanImport(paths: string[]): Promise<ImportFile[]>;
	/**
	 * Copies the files into the library in the background, on all processor threads. With
	 * `moveOriginals`, each original is removed once its copy has been checked.
	 */
	startImport(files: { path: string; game: string }[], moveOriginals: boolean): Promise<void>;
	onImportProgress(handler: (progress: ImportProgress) => void): Promise<() => void>;
	onImportDone(handler: (done: ImportDone) => void): Promise<() => void>;
	/** Files and folders dragged onto the window from Explorer. */
	onFileDrop(handler: (event: FileDrop) => void): Promise<() => void>;
	onLoadTestProgress(handler: (progress: LoadProgress) => void): Promise<() => void>;
}

/** Tauri command and event names, in the order of the `Api` methods. */
export const COMMANDS = {
	getSettings: 'get_settings',
	setSettings: 'set_settings',
	getStatus: 'get_status',
	getSystemInfo: 'get_system_info',
	startRecording: 'start_recording',
	stopRecording: 'stop_recording',
	saveClip: 'save_clip',
	takeScreenshot: 'take_screenshot',
	listMonitors: 'list_monitors',
	listWindows: 'list_windows',
	listRunningGames: 'list_running_games',
	listAudioApps: 'list_audio_apps',
	listMicrophones: 'list_microphones',
	listCaptureSources: 'list_capture_sources',
	getPreview: 'get_preview',
	estimateUsage: 'estimate_usage',
	runBenchmark: 'run_benchmark',
	runLoadTest: 'run_load_test',
	getDefaultSettings: 'get_default_settings',
	getStorageUsage: 'get_storage_usage',
	cleanUpStorage: 'clean_up_storage',
	isFirstRun: 'is_first_run',
	finishOnboarding: 'finish_onboarding',
	startFileRecording: 'start_file_recording',
	stopFileRecording: 'stop_file_recording',
	pauseFileRecording: 'pause_file_recording',
	cancelBenchmark: 'cancel_benchmark',
	videoSettingsCode: 'video_settings_code',
	readVideoSettingsCode: 'read_video_settings_code',
	openSettingsFolder: 'open_settings_folder',
	listClips: 'list_clips',
	deleteClip: 'delete_clip',
	deleteClips: 'delete_clips',
	renameClip: 'rename_clip',
	revealClip: 'reveal_clip',
	exportClip: 'export_clip',
	copyClipsToClipboard: 'copy_clips_to_clipboard',
	dragClipsOut: 'drag_clips_out',
	pickVideos: 'pick_videos',
	listImportSources: 'list_import_sources',
	scanImport: 'scan_import',
	startImport: 'start_import',
	listProcessing: 'list_processing',
	shrinkEstimate: 'shrink_estimate',
	shrinkNow: 'shrink_now',
	compressClips: 'compress_clips',
	stopCompressing: 'stop_compressing',
	burnIn: 'burn_in',
	stopBurnIn: 'stop_burn_in',
	getTrackAudio: 'get_track_audio',
	getWaveforms: 'get_waveforms',
	listAlbums: 'list_albums',
	createAlbum: 'create_album',
	renameAlbum: 'rename_album',
	deleteAlbum: 'delete_album',
	setAlbumHidden: 'set_album_hidden',
	addToAlbum: 'add_to_album',
	removeFromAlbum: 'remove_from_album',
	pickFolder: 'pick_folder',
	pickProgram: 'pick_program',
	pickLogo: 'pick_logo',
	listCameras: 'list_cameras',
	getCameraVideo: 'get_camera_video',
	getAutoClipLog: 'get_auto_clip_log',
	getInputTrack: 'get_input_track',
	setInputCapture: 'set_input_capture',
	installCs2Config: 'install_cs2_config',
	getCs2Config: 'get_cs2_config'
} as const;

export const EVENTS = {
	status: 'status',
	clipSaved: 'clip-saved',
	clipSaving: 'clip-saving',
	clipFailed: 'clip-failed',
	exportProgress: 'export-progress',
	clipsChanged: 'clips-changed',
	albumsChanged: 'albums-changed',
	benchmarkProgress: 'benchmark-progress',
	recordingSaved: 'recording-saved',
	recordingFinishing: 'recording-finishing',
	processingStarted: 'processing-started',
	shrinkResult: 'shrink-result',
	burnResult: 'burn-result',
	storageCleaned: 'storage-cleaned',
	lowDisk: 'low-disk',
	importProgress: 'import-progress',
	importDone: 'import-done',
	loadTestProgress: 'loadtest-progress',
	autoClipLog: 'auto-clip-log'
} as const;

/**
 * Argument names for commands that take more than one value (Tauri passes arguments by name,
 * camelCase on this side): getPreview { target, maxWidth }, renameClip { path, newName },
 * renameAlbum { id, name }, setAlbumHidden { id, hideFromClips }, addToAlbum { id, paths },
 * removeFromAlbum { id, paths }, deleteClips { paths }, estimateUsage { settings },
 * exportClip { request }, setSettings { settings }, getTrackAudio { path }, createAlbum { name },
 * deleteAlbum { id }.
 */
