<script lang="ts">
	import BufferingIndicator from '$lib/components/app/BufferingIndicator.svelte';
	import ScrubPreview from '$lib/components/app/ScrubPreview.svelte';
	import ClipOverlays from '$lib/components/app/overlay/ClipOverlays.svelte';
	import { api } from '$lib/api';
	import { nearBookmark, stepBookmark } from '$lib/format/bookmarks';
	import { onMount, untrack } from 'svelte';
	import PlayIcon from '@lucide/svelte/icons/play';
	import PauseIcon from '@lucide/svelte/icons/pause';
	import SkipBackIcon from '@lucide/svelte/icons/skip-back';
	import SkipForwardIcon from '@lucide/svelte/icons/skip-forward';
	import Volume2Icon from '@lucide/svelte/icons/volume-2';
	import VolumeXIcon from '@lucide/svelte/icons/volume-x';
	import ScissorsIcon from '@lucide/svelte/icons/scissors';
	import MaximizeIcon from '@lucide/svelte/icons/maximize';
	import MinimizeIcon from '@lucide/svelte/icons/minimize';
	import { Button } from '$lib/components/ui/button';
	import { Kbd } from '$lib/components/ui/kbd';
	import { Slider } from '$lib/components/ui/slider';
	import * as Empty from '$lib/components/ui/empty';
	import ClipHeader from '$lib/components/app/ClipHeader.svelte';
	import IconButton from '$lib/components/app/IconButton.svelte';
	import { Playback } from '$lib/editor/playback.svelte';
	import {
		app,
		backToLibrary,
		browsePosition,
		browseStep,
		deleteWithUndo,
		openInEditor,
		playerClip,
		swallowNextClick,
		takeDoubleClick
	} from '$lib/stores/app.svelte';
	import { formatTimecode } from '$lib/format/format';

	/**
	 * Watching clips: plays the clip full size with only the controls that matter for
	 * looking through a library. Editing is one button (or E) away.
	 */

	const clip = $derived(playerClip());
	const seekSecs = $derived(app.settings?.player.seekSecs ?? 5);

	// One playback controller per video file; a new clip gets a fresh one.
	let current: { url: string; playback: Playback } | null = null;
	const playback = $derived.by(() => {
		if (!clip) return null;
		if (current?.url !== clip.videoUrl) {
			current = { url: clip.videoUrl, playback: new Playback(clip.videoUrl, clip.durationSecs, clip.fps) };
		}
		return current.playback;
	});

	let video = $state<HTMLVideoElement | null>(null);
	let stageWidth = $state(0);
	let stageHeight = $state(0);

	// Volume carries over from clip to clip and between visits.
	const VOLUME_KEY = 'umbra.player.volume.v1';
	function savedVolume(): { volume: number; muted: boolean } {
		try {
			const raw = JSON.parse(localStorage.getItem(VOLUME_KEY) ?? '{}') as { volume?: number; muted?: boolean };
			return { volume: typeof raw.volume === 'number' ? raw.volume : 1, muted: raw.muted === true };
		} catch {
			return { volume: 1, muted: false };
		}
	}
	function saveVolume(volume: number, muted: boolean) {
		try {
			localStorage.setItem(VOLUME_KEY, JSON.stringify({ volume, muted }));
		} catch {
			/* storage may be unavailable */
		}
	}

	$effect(() => {
		const pb = playback;
		if (!pb) return;
		const saved = savedVolume();
		pb.setVolume(saved.volume);
		pb.setMuted(saved.muted);
		// The mock has no video element; its clock starts right away.
		if (pb.simulated) pb.play();
		return () => pb.dispose();
	});

	$effect(() => {
		const pb = playback;
		pb?.attach(video);
		return () => pb?.attach(null);
	});

	// Tracks only on their own (the microphone) are not in the combined track the video plays,
	// so they are played next to it once their audio is ready; until then the video's sound
	// plays as it is.
	$effect(() => {
		const pb = playback;
		const shown = untrack(() => clip);
		if (!pb || !shown || pb.simulated) return;
		const mix = shown.audioTracks.find((t) => t.role === 'mix');
		const own = shown.audioTracks.filter((t) => t.role === 'separate');
		if (!mix || own.length === 0) {
			pb.audioMode = 'video';
			return;
		}
		let cancelled = false;
		void (async () => {
			try {
				const files = await api.getTrackAudio(shown.path);
				if (cancelled) return;
				const wanted = files.filter(
					(f) => f.kind === 'track' && (f.index === mix.index || own.some((t) => t.index === f.index))
				);
				await pb.useTracks(wanted);
				pb.setTrackVolumes(Object.fromEntries(wanted.map((f) => [f.index, 1])));
			} catch {
				pb.audioMode = 'video';
			}
		})();
		return () => {
			cancelled = true;
		};
	});

	const position = $derived.by(() => {
		void app.playerPath;
		void app.clips;
		return browsePosition();
	});

	function setVolume(v: number) {
		if (!playback) return;
		playback.setVolume(v);
		if (playback.muted && v > 0) playback.setMuted(false);
		saveVolume(playback.volume, playback.muted);
	}
	function toggleMute() {
		if (!playback) return;
		playback.setMuted(!playback.muted);
		saveVolume(playback.volume, playback.muted);
	}

	function edit() {
		if (clip) openInEditor(clip.path, playback?.time);
	}

	// ---------- Scrub bar ----------

	let bar = $state<HTMLDivElement | null>(null);
	let scrubbing = $state(false);
	let hoverX = $state<number | null>(null);
	let barWidth = $state(0);
	let resumeAfterScrub = false;

	const duration = $derived(Math.max(0.001, playback?.duration ?? 0));
	const fraction = $derived(playback ? Math.min(1, Math.max(0, playback.time / duration)) : 0);

	const bookmarks = $derived(clip?.bookmarks ?? []);

	function timeAt(clientX: number): number {
		const rect = bar!.getBoundingClientRect();
		const t = Math.min(1, Math.max(0, (clientX - rect.left) / rect.width)) * duration;
		// A press within a few pixels of a bookmark lands exactly on it.
		return nearBookmark(bookmarks, t, (6 / Math.max(1, rect.width)) * duration) ?? t;
	}
	function scrubDown(e: PointerEvent) {
		if (e.button !== 0 || !bar || !playback) return;
		bar.setPointerCapture(e.pointerId);
		scrubbing = true;
		resumeAfterScrub = playback.playing;
		playback.pause();
		playback.seek(timeAt(e.clientX));
	}
	function scrubMove(e: PointerEvent) {
		if (!bar) return;
		const rect = bar.getBoundingClientRect();
		hoverX = Math.min(rect.width, Math.max(0, e.clientX - rect.left));
		if (scrubbing && playback) playback.seek(timeAt(e.clientX));
	}
	function scrubUp(e: PointerEvent) {
		if (!scrubbing) return;
		scrubbing = false;
		bar?.releasePointerCapture?.(e.pointerId);
		if (resumeAfterScrub) playback?.play();
	}

	// ---------- Keyboard ----------

	function onKeydown(e: KeyboardEvent) {
		if (!playback || !clip || e.defaultPrevented || e.altKey || e.metaKey) return;
		const target = e.target as HTMLElement | null;
		if (
			target?.closest(
				'input, textarea, select, [contenteditable="true"], [role="combobox"], [role="listbox"], [role="menu"], [role="dialog"], [role="alertdialog"]'
			)
		)
			return;
		const onControl = !!target?.closest('button, a, [role="slider"], [role="switch"], [role="checkbox"]');

		if (e.ctrlKey) {
			if (e.code === 'ArrowLeft' || e.code === 'ArrowRight') {
				e.preventDefault();
				browseStep(e.code === 'ArrowLeft' ? -1 : 1);
			}
			return;
		}
		switch (e.code) {
			case 'Space':
			case 'KeyK':
				if (e.code === 'Space' && onControl) return;
				e.preventDefault();
				playback.toggle();
				break;
			case 'ArrowLeft':
			case 'ArrowRight': {
				if (target?.closest('[role="slider"]')) return;
				e.preventDefault();
				const dir = e.code === 'ArrowLeft' ? -1 : 1;
				if (e.shiftKey) playback.step(dir);
				else playback.skip(dir * seekSecs);
				break;
			}
			case 'Home':
				e.preventDefault();
				playback.seek(0);
				break;
			case 'BracketLeft':
			case 'BracketRight': {
				const to = stepBookmark(bookmarks, playback.time, e.code === 'BracketLeft' ? -1 : 1);
				if (to === null) return;
				e.preventDefault();
				playback.seek(to);
				break;
			}
			case 'End':
				e.preventDefault();
				playback.seek(playback.duration);
				break;
			case 'KeyM':
				e.preventDefault();
				toggleMute();
				break;
			case 'KeyE':
			case 'Enter':
			case 'NumpadEnter':
				if (e.code !== 'KeyE' && onControl) return;
				e.preventDefault();
				edit();
				break;
			case 'Delete':
				e.preventDefault();
				deleteWithUndo(clip.path);
				break;
			case 'KeyF':
				e.preventDefault();
				toggleFullscreen();
				break;
			case 'Escape':
				e.preventDefault();
				// Out of full screen first; only then back to the library.
				if (document.fullscreenElement) void document.exitFullscreen();
				else backToLibrary();
				break;
		}
	}

	// ---------- Full screen ----------

	/** The picture and its controls, which go full screen together. */
	let frame = $state<HTMLDivElement | null>(null);
	let fullscreen = $state(false);
	/** No pointer movement for a moment while playing in full screen: controls and cursor hide. */
	let idle = $state(false);
	let idleTimer: ReturnType<typeof setTimeout> | null = null;

	function toggleFullscreen() {
		if (document.fullscreenElement) void document.exitFullscreen();
		else void frame?.requestFullscreen().catch(() => {});
	}

	function wake() {
		idle = false;
		if (idleTimer) clearTimeout(idleTimer);
		idleTimer = setTimeout(() => (idle = true), 2500);
	}

	onMount(() => {
		const onChange = () => {
			fullscreen = document.fullscreenElement !== null && document.fullscreenElement === frame;
			if (fullscreen) wake();
		};
		document.addEventListener('fullscreenchange', onChange);
		return () => {
			document.removeEventListener('fullscreenchange', onChange);
			if (idleTimer) clearTimeout(idleTimer);
			// Leaving the player leaves full screen too.
			if (document.fullscreenElement) void document.exitFullscreen();
		};
	});

	onMount(() => {
		// The second click of a double-click in the library lands here: open the editor instead.
		const onPointerDown = (e: PointerEvent) => {
			const path = takeDoubleClick(e);
			if (!path) return;
			e.preventDefault();
			e.stopPropagation();
			swallowNextClick();
			openInEditor(path, 0);
		};
		window.addEventListener('pointerdown', onPointerDown, true);
		return () => window.removeEventListener('pointerdown', onPointerDown, true);
	});
</script>

<svelte:window onkeydown={onKeydown} />

{#if clip && playback}
	<section class="flex min-h-0 flex-1 flex-col" aria-label="Clip player">
		<ClipHeader {clip}>
			{#snippet tools()}
				{#if position.count > 1 && position.index >= 0}
					<span class="tnum text-xs text-subtle max-[1099px]:hidden">{position.index + 1} of {position.count}</span>
				{/if}
				<Button size="sm" onclick={edit} aria-keyshortcuts="E" title="Open in the editor at this frame (E)">
					<ScissorsIcon data-icon="inline-start" />
					Edit
					<Kbd class="ml-1">E</Kbd>
				</Button>
			{/snippet}
		</ClipHeader>

		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div
			bind:this={frame}
			class="player-frame relative flex min-h-0 flex-1 flex-col bg-black"
			class:fullscreen
			class:idle={fullscreen && idle && playback.playing}
			onpointermove={wake}
			onpointerdown={wake}
		>
		<div
			class="stage relative flex min-h-0 flex-1 items-center justify-center bg-black"
			bind:clientWidth={stageWidth}
			bind:clientHeight={stageHeight}
		>
			{#if playback.simulated}
				<!-- Mock backend: no media file, so the frame is the thumbnail and time is a clock. -->
				<button
					type="button"
					class="absolute inset-0 flex cursor-default items-center justify-center"
					aria-label={playback.playing ? 'Pause' : 'Play'}
					onclick={() => playback.toggle()}
				>
					{#if clip.thumbnailUrl}
						<img src={clip.thumbnailUrl} alt="" class="size-full object-contain" draggable="false" />
					{/if}
					<span class="absolute bottom-3 left-3 rounded-xs bg-black/75 px-1 text-xs text-muted-foreground">
						Simulated playback (mock data, no video file)
					</span>
				</button>
			{:else}
				{#key clip.videoUrl}
					<!-- svelte-ignore a11y_media_has_caption -->
					<video
						bind:this={video}
						src={clip.videoUrl}
						class="absolute inset-0 size-full bg-black object-contain"
						preload="auto"
						playsinline
						disablepictureinpicture
						disableremoteplayback
						aria-label="Clip"
						onloadedmetadata={() => {
							playback.onMetadata();
							playback.play();
						}}
						onerror={() => playback.onError()}
						onclick={() => playback.toggle()}
						ondblclick={toggleFullscreen}
					></video>
				{/key}
			{/if}

			<ClipOverlays {clip} {playback} width={stageWidth} height={stageHeight} />

			{#if playback.error}
				<div class="absolute inset-0 flex items-center justify-center bg-black/85 p-6 text-center">
					<p class="max-w-xs text-sm text-muted-foreground" role="alert">{playback.error}</p>
				</div>
			{:else if !playback.ready}
				<div class="absolute inset-0 flex items-center justify-center text-sm text-subtle" role="status">Loading video</div>
			{:else}
				<BufferingIndicator active={playback.buffering} />
			{/if}
		</div>

		<div class="controls shrink-0 border-t border-line bg-panel px-3 pb-1">
			<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
			<div
				bind:this={bar}
				bind:clientWidth={barWidth}
				class="scrub group relative h-5 cursor-pointer touch-none select-none"
				role="slider"
				tabindex="-1"
				aria-label="Position"
				aria-valuemin={0}
				aria-valuemax={duration}
				aria-valuenow={playback.time}
				aria-valuetext={formatTimecode(playback.time)}
				onpointerdown={scrubDown}
				onpointermove={scrubMove}
				onpointerup={scrubUp}
				onpointercancel={scrubUp}
				onpointerleave={() => (hoverX = null)}
			>
				<span class="track absolute inset-x-0 top-1/2 h-1 -translate-y-1/2 overflow-hidden rounded-full bg-selected">
					<span class="absolute inset-0 origin-left bg-foreground" style:transform="scaleX({fraction})"></span>
				</span>
				{#each bookmarks as mark, i (i)}
					<span
						class="mark"
						style:left="{(mark / duration) * barWidth}px"
						title="Bookmark at {formatTimecode(mark)}"
						aria-hidden="true"
					></span>
				{/each}
				<span class="knob" style:left="{fraction * barWidth}px" aria-hidden="true"></span>
				{#if clip.videoUrl.startsWith('mock:')}
					{#if hoverX !== null && !scrubbing}
						<span class="hover-time tnum" style:left="{hoverX}px" aria-hidden="true">
							{formatTimecode((hoverX / Math.max(1, barWidth)) * duration)}
						</span>
					{/if}
				{:else}
					<ScrubPreview
						url={clip.videoUrl}
						time={hoverX !== null && !scrubbing ? (hoverX / Math.max(1, barWidth)) * duration : null}
						x={hoverX ?? 0}
						{barWidth}
					/>
				{/if}
			</div>

			<div class="flex h-10 items-center gap-2" role="toolbar" aria-label="Player controls">
				<IconButton
					label="Previous clip"
					shortcut="Ctrl+←"
					icon={SkipBackIcon}
					disabled={position.index <= 0}
					onclick={() => browseStep(-1)}
				/>
				<IconButton
					label={playback.playing ? 'Pause' : 'Play'}
					shortcut="Space"
					icon={playback.playing ? PauseIcon : PlayIcon}
					variant="secondary"
					onclick={() => playback.toggle()}
				/>
				<IconButton
					label="Next clip"
					shortcut="Ctrl+→"
					icon={SkipForwardIcon}
					disabled={position.index < 0 || position.index >= position.count - 1}
					onclick={() => browseStep(1)}
				/>
				<div class="mx-2 flex items-baseline gap-2 whitespace-nowrap">
					<span class="tnum min-w-[72px] text-base font-medium text-foreground">{formatTimecode(playback.time)}</span>
					<span class="tnum text-xs text-subtle">/ {formatTimecode(playback.duration)}</span>
					<span class="tnum text-xs text-subtle max-[899px]:hidden">
						Frame {playback.frame.toLocaleString('en-US')} · {Math.round(clip.fps)} fps
					</span>
				</div>
				<div class="ml-auto flex items-center gap-1">
					<IconButton
						label={playback.muted ? 'Unmute' : 'Mute'}
						shortcut="M"
						icon={playback.muted ? VolumeXIcon : Volume2Icon}
						onclick={toggleMute}
					/>
					<div class="w-24 px-1">
						<Slider
							type="single"
							min={0}
							max={1}
							step={0.01}
							value={playback.muted ? 0 : playback.volume}
							onValueChange={setVolume}
							aria-label="Volume"
						/>
					</div>
					<IconButton
						label={fullscreen ? 'Leave full screen' : 'Full screen'}
						shortcut="F"
						icon={fullscreen ? MinimizeIcon : MaximizeIcon}
						onclick={toggleFullscreen}
					/>
				</div>
			</div>
		</div>
		</div>
	</section>
{:else}
	<Empty.Root class="flex-1 border-0">
		<Empty.Header>
			<Empty.Title>No clip is open</Empty.Title>
			<Empty.Description>Pick a clip in the library to watch it.</Empty.Description>
		</Empty.Header>
		<Empty.Content>
			<Button variant="outline" onclick={backToLibrary}>Go to clips</Button>
		</Empty.Content>
	</Empty.Root>
{/if}

<style>
	/* Full screen: the picture fills the screen and the controls float over its bottom edge. */
	.player-frame.fullscreen .controls {
		position: absolute;
		inset-inline: 0;
		bottom: 0;
		border-top: 0;
		background: linear-gradient(to top, rgb(0 0 0 / 0.85), rgb(0 0 0 / 0.55) 70%, transparent);
		padding-top: 24px;
		transition: opacity 200ms var(--ease-out);
	}
	.player-frame.idle {
		cursor: none;
	}
	.player-frame.idle .controls {
		opacity: 0;
		pointer-events: none;
	}
	@media (prefers-reduced-motion: reduce) {
		.player-frame.fullscreen .controls {
			transition: none;
		}
	}
	.knob {
		position: absolute;
		top: 50%;
		width: 12px;
		height: 12px;
		border-radius: 9999px;
		background: var(--foreground);
		transform: translate(-50%, -50%) scale(0.75);
		opacity: 0;
		transition:
			opacity var(--dur-fast) var(--ease-out),
			transform var(--dur-fast) var(--ease-out);
		pointer-events: none;
	}
	.scrub:hover .knob,
	.scrub:active .knob {
		opacity: 1;
		transform: translate(-50%, -50%) scale(1);
	}
	.hover-time {
		position: absolute;
		bottom: 100%;
		transform: translateX(-50%);
		padding: 0 4px;
		border-radius: 2px;
		background: var(--foreground);
		color: var(--background);
		font-size: 12px;
		line-height: 16px;
		white-space: nowrap;
		pointer-events: none;
	}
	/* Bookmarks: moments marked with the clip hotkey while recording. */
	.mark {
		position: absolute;
		top: 50%;
		width: 3px;
		height: 12px;
		margin-left: -1.5px;
		transform: translateY(-50%);
		border-radius: 1px;
		background: var(--mark);
		pointer-events: none;
	}
</style>
