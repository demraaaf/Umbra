import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import {
	COMMANDS,
	EVENTS,
	type Api,
	type AutoClipLogEntry,
	type BenchmarkProgress,
	type Cleanup,
	type Clip,
	type LoadProgress,
	type LowDisk,
	type ImportDone,
	type ImportProgress,
	type ExportProgress,
	type RecordingFinishing,
	type ShrinkResult,
	type BurnResult,
	type RecordingSaved,
	type Status
} from './types';

/**
 * The real backend. Command and event names come from `COMMANDS` / `EVENTS` in types.ts.
 *
 * Tauri maps camelCase argument keys on the JS side to snake_case parameters in Rust, so
 * `{ newName }` arrives as `new_name`. Argument names follow the note at the end of types.ts.
 */
export function createTauriApi(): Api {
	return {
		getSettings: () => invoke(COMMANDS.getSettings),
		setSettings: (settings) => invoke(COMMANDS.setSettings, { settings }),
		getStatus: () => invoke(COMMANDS.getStatus),
		getSystemInfo: () => invoke(COMMANDS.getSystemInfo),

		startRecording: () => invoke(COMMANDS.startRecording),
		stopRecording: () => invoke(COMMANDS.stopRecording),
		saveClip: () => invoke(COMMANDS.saveClip),
		takeScreenshot: () => invoke(COMMANDS.takeScreenshot),

		listMonitors: () => invoke(COMMANDS.listMonitors),
		listWindows: () => invoke(COMMANDS.listWindows),
		listRunningGames: () => invoke(COMMANDS.listRunningGames),
		listAudioApps: () => invoke(COMMANDS.listAudioApps),
		listMicrophones: () => invoke(COMMANDS.listMicrophones),
		listCaptureSources: () => invoke(COMMANDS.listCaptureSources),
		getPreview: (target, maxWidth) => invoke(COMMANDS.getPreview, { target, maxWidth }),
		estimateUsage: (settings) => invoke(COMMANDS.estimateUsage, { settings }),
		runBenchmark: (answers) => invoke(COMMANDS.runBenchmark, { answers }),
		runLoadTest: (presetId) => invoke(COMMANDS.runLoadTest, { presetId }),
		getDefaultSettings: () => invoke(COMMANDS.getDefaultSettings),
		getStorageUsage: () => invoke(COMMANDS.getStorageUsage),
		cleanUpStorage: () => invoke(COMMANDS.cleanUpStorage),
		isFirstRun: () => invoke(COMMANDS.isFirstRun),
		finishOnboarding: () => invoke(COMMANDS.finishOnboarding),
		startFileRecording: (kind, source) => invoke(COMMANDS.startFileRecording, { kind, source: source ?? null }),
		stopFileRecording: () => invoke(COMMANDS.stopFileRecording),
		pauseFileRecording: (paused) => invoke(COMMANDS.pauseFileRecording, { paused }),
		cancelBenchmark: () => invoke(COMMANDS.cancelBenchmark),
		videoSettingsCode: (video) => invoke(COMMANDS.videoSettingsCode, { video }),
		readVideoSettingsCode: (code) => invoke(COMMANDS.readVideoSettingsCode, { code }),
		openSettingsFolder: () => invoke(COMMANDS.openSettingsFolder),

		listClips: () => invoke(COMMANDS.listClips),
		deleteClip: (path) => invoke(COMMANDS.deleteClip, { path }),
		deleteClips: (paths) => invoke(COMMANDS.deleteClips, { paths }),
		renameClip: (path, newName) => invoke(COMMANDS.renameClip, { path, newName }),
		revealClip: (path) => invoke(COMMANDS.revealClip, { path }),
		exportClip: (request) => invoke(COMMANDS.exportClip, { request }),
		dragClipsOut: (paths) => invoke(COMMANDS.dragClipsOut, { paths }),
		copyClipsToClipboard: (paths) => invoke(COMMANDS.copyClipsToClipboard, { paths }),
		getTrackAudio: (path) => invoke(COMMANDS.getTrackAudio, { path }),
		getCameraVideo: (path) => invoke(COMMANDS.getCameraVideo, { path }),
		getWaveforms: (path) => invoke(COMMANDS.getWaveforms, { path }),

		listAlbums: () => invoke(COMMANDS.listAlbums),
		createAlbum: (name) => invoke(COMMANDS.createAlbum, { name }),
		renameAlbum: (id, name) => invoke(COMMANDS.renameAlbum, { id, name }),
		deleteAlbum: (id) => invoke(COMMANDS.deleteAlbum, { id }),
		setAlbumHidden: (id, hideFromClips) => invoke(COMMANDS.setAlbumHidden, { id, hideFromClips }),
		addToAlbum: (id, paths) => invoke(COMMANDS.addToAlbum, { id, paths }),
		removeFromAlbum: (id, paths) => invoke(COMMANDS.removeFromAlbum, { id, paths }),

		pickFolder: () => invoke(COMMANDS.pickFolder),
		pickProgram: () => invoke(COMMANDS.pickProgram),
		pickLogo: () => invoke(COMMANDS.pickLogo),
		listCameras: () => invoke(COMMANDS.listCameras),
		fileUrl: async (path) => convertFileSrc(path),

		onStatus: (handler) => listen<Status>(EVENTS.status, (event) => handler(event.payload)),
		onClipSaved: (handler) => listen<Clip>(EVENTS.clipSaved, (event) => handler(event.payload)),
		onClipSaving: (handler) =>
			listen<{ game: string; waitSecs: number; lengthSecs: number }>(EVENTS.clipSaving, (event) => handler(event.payload)),
		onClipFailed: (handler) => listen<{ error: string }>(EVENTS.clipFailed, (event) => handler(event.payload)),
		onExportProgress: (handler) =>
			listen<ExportProgress>(EVENTS.exportProgress, (event) => handler(event.payload)),
		onClipsChanged: (handler) => listen(EVENTS.clipsChanged, () => handler()),
		onAlbumsChanged: (handler) => listen(EVENTS.albumsChanged, () => handler()),
		onLoadTestProgress: (handler) =>
			listen<LoadProgress>(EVENTS.loadTestProgress, (event) => handler(event.payload)),
		onStorageCleaned: (handler) => listen<Cleanup>(EVENTS.storageCleaned, (event) => handler(event.payload)),
		onLowDisk: (handler) => listen<LowDisk>(EVENTS.lowDisk, (event) => handler(event.payload)),
		getAutoClipLog: () => invoke(COMMANDS.getAutoClipLog),
		getInputTrack: (path) => invoke(COMMANDS.getInputTrack, { path }),
		setInputCapture: (on) => invoke(COMMANDS.setInputCapture, { on }),
		onAutoClipLog: (handler) => listen<AutoClipLogEntry>(EVENTS.autoClipLog, (event) => handler(event.payload)),
		installCs2Config: () => invoke(COMMANDS.installCs2Config),
		getCs2Config: () => invoke(COMMANDS.getCs2Config),
		pickVideos: () => invoke(COMMANDS.pickVideos),
		listImportSources: () => invoke(COMMANDS.listImportSources),
		scanImport: (paths) => invoke(COMMANDS.scanImport, { paths }),
		startImport: (files, moveOriginals) => invoke(COMMANDS.startImport, { files, moveOriginals }),
		onImportProgress: (handler) => listen<ImportProgress>(EVENTS.importProgress, (event) => handler(event.payload)),
		onImportDone: (handler) => listen<ImportDone>(EVENTS.importDone, (event) => handler(event.payload)),
		onFileDrop: (handler) =>
			getCurrentWebview().onDragDropEvent((event) => {
				const e = event.payload;
				if (e.type === 'enter' || e.type === 'drop') handler({ type: e.type, paths: e.paths });
				else handler({ type: e.type });
			}),
		listProcessing: () => invoke(COMMANDS.listProcessing),
		shrinkEstimate: () => invoke(COMMANDS.shrinkEstimate),
		shrinkNow: () => invoke(COMMANDS.shrinkNow),
		compressClips: (paths, quality) => invoke(COMMANDS.compressClips, { paths, quality: quality ?? null }),
		stopCompressing: (paths) => invoke(COMMANDS.stopCompressing, { paths }),
		onShrinkResult: (handler) => listen<ShrinkResult>(EVENTS.shrinkResult, (event) => handler(event.payload)),
		burnIn: (paths, layers) => invoke(COMMANDS.burnIn, { paths, layers }),
		stopBurnIn: (paths) => invoke(COMMANDS.stopBurnIn, { paths }),
		onBurnResult: (handler) => listen<BurnResult>(EVENTS.burnResult, (event) => handler(event.payload)),
		onProcessingStarted: (handler) => listen(EVENTS.processingStarted, () => handler()),
		onRecordingFinishing: (handler) =>
			listen<RecordingFinishing>(EVENTS.recordingFinishing, (event) => handler(event.payload)),
		onRecordingSaved: (handler) =>
			listen<RecordingSaved>(EVENTS.recordingSaved, (event) => handler(event.payload)),
		onBenchmarkProgress: (handler) =>
			listen<BenchmarkProgress>(EVENTS.benchmarkProgress, (event) => handler(event.payload))
	};
}
