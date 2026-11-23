<script lang="ts">
	import { onMount } from 'svelte';
	import { ContextMenu } from 'bits-ui';
	import CheckIcon from '@lucide/svelte/icons/check';
	import GaugeIcon from '@lucide/svelte/icons/gauge';
	import ScissorsIcon from '@lucide/svelte/icons/scissors';
	import { SPEEDS, speedLabel, type EditSession } from '$lib/editor/session.svelte';
	import { clamp, formatDuration, formatTimecode } from '$lib/format/format';
	import WaveRow from './WaveRow.svelte';
	import CameraRow from './CameraRow.svelte';

	/** `showWaves` adds one waveform row per audio track under the clip strip. */
	let { session, showWaves = false }: { session: EditSession; showWaves?: boolean } = $props();

	const playback = $derived(session.playback);

	const EDGE = 12; // px of room either side so handles at 0 and at the end stay grabbable

	let width = $state(0);
	let lane = $state<HTMLDivElement | null>(null);
	let dragging = $state<'in' | 'out' | null>(null);
	let scrubbing = $state(false);
	/** Shift and a drag on the timeline: picking a selection instead of moving the playhead. */
	let selecting = $state(false);
	let focusedHandle = $state<'in' | 'out' | null>(null);
	/** The cut point being dragged, by index. */
	let draggingSplit = $state<number | null>(null);
	/** Where a drag snapped to, drawn as a guide while it holds. */
	let snappedAt = $state<number | null>(null);

	const SNAP_PX = 8;

	const duration = $derived(Math.max(0.001, playback.duration));
	const span = $derived(Math.max(0.001, session.visibleSpan));
	const start = $derived(session.visibleStart);
	const pxPerSec = $derived(width > 0 ? width / span : 0);

	const xOf = (t: number) => (t - start) * pxPerSec;
	const tOf = (x: number) => start + x / (pxPerSec || 1);
	const snap = (t: number) => clamp(Math.round(t * session.clip.fps) / session.clip.fps, 0, duration);

	/**
	 * Magnetic snapping: within a few pixels of a cut point, a bookmark, a trim end or the edge
	 * of a part taken out, a drag lands on it. `dragged` names what is being dragged, so it does
	 * not stick to itself. (Every drag moves the playhead along, so the playhead is not a point
	 * to stick to.) Holding Alt drags freely.
	 */
	type Dragged = 'playhead' | 'in' | 'out' | number;
	function magnetize(t: number, dragged: Dragged, free: boolean): number {
		snappedAt = null;
		if (!session.magnet || free || pxPerSec <= 0) return t;
		const points = [
			0,
			duration,
			...session.clip.bookmarks,
			...session.cuts.flatMap((c) => [c.start, c.end]),
			...session.splits.filter((_, i) => i !== dragged)
		];
		if (dragged !== 'in') points.push(session.inSecs);
		if (dragged !== 'out') points.push(session.outSecs);
		const reach = SNAP_PX / pxPerSec;
		let best: number | null = null;
		for (const p of points) {
			if (Math.abs(p - t) <= reach && (best === null || Math.abs(p - t) < Math.abs(best - t))) best = p;
		}
		if (best === null) return t;
		snappedAt = best;
		return best;
	}

	const STEPS = [0.05, 0.1, 0.25, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600];
	const SUBDIVISIONS = 4;

	const ticks = $derived.by(() => {
		if (pxPerSec <= 0) return [];
		const step = STEPS.find((s) => s * pxPerSec >= 88) ?? STEPS[STEPS.length - 1];
		const minor = step / SUBDIVISIONS;
		const list: { t: number; x: number; label: string | null }[] = [];
		const first = Math.floor((start - EDGE / pxPerSec) / minor) * minor;
		const last = start + span + EDGE / pxPerSec;
		for (let t = first, i = 0; t <= last + 1e-9 && i < 600; t += minor, i++) {
			if (t < -1e-9 || t > duration + 1e-9) continue;
			const isMajor = Math.abs(t / step - Math.round(t / step)) < 1e-6;
			if (!isMajor && minor * pxPerSec < 10) continue;
			list.push({ t, x: xOf(t), label: isMajor ? label(t, step) : null });
		}
		return list;
	});

	function label(t: number, step: number): string {
		if (step >= 1) return formatDuration(t);
		const text = formatTimecode(t);
		return step < 0.1 ? text.slice(0, -1) : text.slice(0, -2);
	}

	/** A range as a left and width in px, clipped to the visible lane. */
	function band(a: number, b: number): { left: number; width: number } | null {
		const left = Math.max(xOf(a), -EDGE);
		const right = Math.min(xOf(b), width + EDGE);
		return right > left ? { left, width: right - left } : null;
	}
	const selectionBand = $derived(session.selection ? band(session.selection.start, session.selection.end) : null);
	const segmentBand = $derived(session.segment ? band(session.segment.start, session.segment.end) : null);
	const speedBands = $derived(
		session.speeds
			.map((r, i) => ({ i, r, b: band(r.start, r.end) }))
			.filter((x): x is { i: number; r: { start: number; end: number; speed: number }; b: { left: number; width: number } } => x.b !== null)
	);

	// ---------- Right-click menu ----------

	/** What the menu acts on, worked out when it opens. */
	let menuTarget = $state<{ start: number; end: number } | null>(null);
	let menuSpeed = $state(1);
	const menuItem =
		'flex cursor-default items-center gap-2 rounded-md px-2 py-1.5 text-sm outline-hidden select-none data-highlighted:bg-accent data-highlighted:text-accent-foreground data-disabled:opacity-50 [&_svg]:size-4 [&_svg]:shrink-0';

	/** A right-click outside the selection moves the playhead there first, so the menu acts
	 * on the part that was clicked. */
	function onContextMenu(e: MouseEvent) {
		if (!lane) return;
		const rect = lane.getBoundingClientRect();
		const t = snap(tOf(e.clientX - rect.left));
		const sel = session.selection;
		if (!sel || t < sel.start || t > sel.end) {
			session.clearSelection();
			playback.pause();
			playback.seek(t);
		}
		menuTarget = session.speedTarget();
		menuSpeed = menuTarget ? playback.speedAt((menuTarget.start + menuTarget.end) / 2) : 1;
	}
	const cutBands = $derived(
		session.keptCuts
			.map((c, i) => ({ i, c, b: band(c.start, c.end) }))
			.filter((x): x is { i: number; c: { start: number; end: number }; b: { left: number; width: number } } => x.b !== null)
	);

	$effect(() => {
		if (showWaves) void session.loadWaves();
	});

	const inX = $derived(xOf(session.inSecs));
	const outX = $derived(xOf(session.outSecs));
	const playX = $derived(xOf(playback.time));

	// ---------- Pointer: scrub the playhead ----------

	function timeAt(e: PointerEvent, dragged: Dragged = 'playhead'): number {
		const rect = lane!.getBoundingClientRect();
		return magnetize(snap(tOf(e.clientX - rect.left)), dragged, e.altKey);
	}

	function scrubDown(e: PointerEvent) {
		if (e.button !== 0 || !lane) return;
		lane.setPointerCapture(e.pointerId);
		playback.pause();
		if (e.shiftKey && !session.busy) {
			selecting = true;
			session.beginSelection(timeAt(e));
			return;
		}
		scrubbing = true;
		playback.seek(timeAt(e));
	}
	function scrubMove(e: PointerEvent) {
		if (!lane) return;
		if (selecting) session.dragSelection(timeAt(e));
		else if (scrubbing) playback.seek(timeAt(e));
	}
	function scrubUp(e: PointerEvent) {
		if (!scrubbing && !selecting) return;
		scrubbing = false;
		selecting = false;
		snappedAt = null;
		lane?.releasePointerCapture?.(e.pointerId);
	}

	// ---------- Pointer: drag a trim handle ----------

	function handleDown(e: PointerEvent, which: 'in' | 'out') {
		if (e.button !== 0 || session.busy) return;
		e.preventDefault();
		e.stopPropagation();
		(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
		dragging = which;
		playback.pause();
	}
	function handleMove(e: PointerEvent) {
		if (!dragging || !lane) return;
		const t = timeAt(e, dragging);
		if (dragging === 'in') {
			session.setIn(t);
			playback.seek(session.inSecs);
		} else {
			session.setOut(t);
			playback.seek(session.outSecs);
		}
	}
	function handleUp(e: PointerEvent) {
		if (!dragging) return;
		(e.currentTarget as HTMLElement).releasePointerCapture?.(e.pointerId);
		dragging = null;
		snappedAt = null;
	}

	// ---------- Pointer and keys: cut points ----------

	function splitDown(e: PointerEvent, index: number) {
		if (e.button !== 0 || session.busy) return;
		e.preventDefault();
		e.stopPropagation();
		(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
		draggingSplit = index;
		playback.pause();
		playback.seek(session.splits[index]);
	}
	function splitMove(e: PointerEvent) {
		if (draggingSplit === null || !lane) return;
		session.moveSplit(draggingSplit, timeAt(e, draggingSplit));
		playback.seek(session.splits[draggingSplit]);
	}
	function splitUp(e: PointerEvent) {
		if (draggingSplit === null) return;
		(e.currentTarget as HTMLElement).releasePointerCapture?.(e.pointerId);
		draggingSplit = null;
		snappedAt = null;
	}
	function splitKeydown(e: KeyboardEvent, index: number) {
		if (session.busy) return;
		const frame = session.frameSecs;
		if (e.key === 'Delete' || e.key === 'Backspace') {
			e.preventDefault();
			e.stopPropagation();
			playback.seek(session.splits[index]);
			session.toggleCutAtPlayhead();
			return;
		}
		const delta = e.key === 'ArrowLeft' ? -frame : e.key === 'ArrowRight' ? frame : 0;
		if (!delta) return;
		e.preventDefault();
		e.stopPropagation();
		session.moveSplit(index, session.splits[index] + delta * (e.shiftKey ? 10 : 1));
		playback.seek(session.splits[index]);
	}

	function handleKeydown(e: KeyboardEvent, which: 'in' | 'out') {
		if (session.busy) return;
		const frame = session.frameSecs;
		let delta = 0;
		if (e.key === 'ArrowLeft') delta = -(e.shiftKey ? 1 : e.ctrlKey ? 10 * frame : frame);
		else if (e.key === 'ArrowRight') delta = e.shiftKey ? 1 : e.ctrlKey ? 10 * frame : frame;
		else if (e.key === 'PageDown') delta = -5;
		else if (e.key === 'PageUp') delta = 5;
		else if (e.key === 'Home' || e.key === 'End') {
			e.preventDefault();
			e.stopPropagation();
			const target = e.key === 'Home' ? 0 : duration;
			if (which === 'in') session.setIn(target);
			else session.setOut(target);
			playback.seek(which === 'in' ? session.inSecs : session.outSecs);
			return;
		} else return;
		e.preventDefault();
		e.stopPropagation();
		if (which === 'in') {
			session.setIn(session.inSecs + delta);
			playback.seek(session.inSecs);
		} else {
			session.setOut(session.outSecs + delta);
			playback.seek(session.outSecs);
		}
	}

	// ---------- Zoom and pan ----------

	onMount(() => {
		const el = lane;
		if (!el) return;
		// Wheel handlers must be non-passive to stop the page from scrolling.
		const onWheel = (e: WheelEvent) => {
			const rect = el.getBoundingClientRect();
			if (e.ctrlKey || e.metaKey) {
				e.preventDefault();
				session.zoomAt(e.deltaY < 0 ? 1.25 : 0.8, tOf(e.clientX - rect.left));
			} else if (session.zoomed) {
				e.preventDefault();
				session.panBy(((e.deltaY || e.deltaX) / (pxPerSec || 1)) * 1.2);
			}
		};
		el.addEventListener('wheel', onWheel, { passive: false });
		return () => el.removeEventListener('wheel', onWheel);
	});

	// While zoomed, keep the playhead in view (also lets keyboard frame-stepping travel the clip).
	$effect(() => {
		if (!session.zoomed || dragging || scrubbing || selecting || draggingSplit !== null) return;
		const t = playback.time;
		const s = session.visibleStart;
		const e = s + session.visibleSpan;
		if (t < s || t > e) session.viewStart = clamp(t - session.visibleSpan * 0.15, 0, duration - session.visibleSpan);
	});

	const inRange = (x: number) => x >= -EDGE && x <= width + EDGE;
</script>

<ContextMenu.Root>
<ContextMenu.Trigger>
{#snippet child({ props })}
<div class="overflow-hidden px-3">
	<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
	<div
		{...props}
		bind:this={lane}
		bind:clientWidth={width}
		class="relative touch-none select-none"
		role="group"
		aria-label="Timeline. Drag to move the playhead, Shift and drag to select, right-click for speed and cuts."
		onpointerdown={scrubDown}
		onpointermove={scrubMove}
		onpointerup={scrubUp}
		onpointercancel={scrubUp}
		oncontextmenu={(e) => {
			onContextMenu(e);
			(props.oncontextmenu as ((e: MouseEvent) => void) | undefined)?.(e);
		}}
	>
		<!-- Ruler -->
		<div class="relative h-6 border-b border-line">
			{#each ticks as tick (tick.t)}
				<span class="tick" class:major={tick.label !== null} style:left="{tick.x}px"></span>
				{#if tick.label !== null}
					<span class="tick-label tnum" style:left="{tick.x + 4}px">{tick.label}</span>
				{/if}
			{/each}

			{#each [{ which: 'in' as const, x: inX, t: session.inSecs }, { which: 'out' as const, x: outX, t: session.outSecs }] as h (h.which)}
				{#if (dragging === h.which || focusedHandle === h.which) && inRange(h.x)}
					<span class="chip tnum" class:chip-end={h.which === 'in'} style:left="{h.x}px">{formatTimecode(h.t)}</span>
				{/if}
			{/each}
		</div>

		<!-- Track -->
		<div class="relative h-10 bg-panel">
			<span class="dim" style="left:-{EDGE}px" style:width="{Math.max(0, Math.min(inX, width + EDGE) + EDGE)}px"></span>
			<span class="dim" style:left="{Math.max(outX, -EDGE)}px" style:width="{Math.max(0, width + EDGE - Math.max(outX, -EDGE))}px"></span>
			<span
				class="range"
				style:left="{Math.max(inX, -EDGE)}px"
				style:width="{Math.max(0, Math.min(outX, width + EDGE) - Math.max(inX, -EDGE))}px"
			></span>

			{#if inRange(inX)}
				<div
					class="handle handle-in"
					class:active={dragging === 'in'}
					style:left="{inX}px"
					role="slider"
					tabindex="0"
					aria-label="Trim start handle"
					aria-orientation="horizontal"
					aria-valuemin={0}
					aria-valuemax={session.outSecs}
					aria-valuenow={session.inSecs}
					aria-valuetext={formatTimecode(session.inSecs)}
					onpointerdown={(e) => handleDown(e, 'in')}
					onpointermove={handleMove}
					onpointerup={handleUp}
					onpointercancel={handleUp}
					onkeydown={(e) => handleKeydown(e, 'in')}
					onfocus={() => (focusedHandle = 'in')}
					onblur={() => (focusedHandle = null)}
				></div>
			{/if}
			{#if inRange(outX)}
				<div
					class="handle handle-out"
					class:active={dragging === 'out'}
					style:left="{outX}px"
					role="slider"
					tabindex="0"
					aria-label="Trim end handle"
					aria-orientation="horizontal"
					aria-valuemin={session.inSecs}
					aria-valuemax={duration}
					aria-valuenow={session.outSecs}
					aria-valuetext={formatTimecode(session.outSecs)}
					onpointerdown={(e) => handleDown(e, 'out')}
					onpointermove={handleMove}
					onpointerup={handleUp}
					onpointercancel={handleUp}
					onkeydown={(e) => handleKeydown(e, 'out')}
					onfocus={() => (focusedHandle = 'out')}
					onblur={() => (focusedHandle = null)}
				></div>
			{/if}
		</div>

		{#if session.cameraPlacement}
			<CameraRow {session} {start} {span} {width} />
		{/if}

		{#if showWaves}
			{#if session.waves}
				{#each session.clip.audioTracks as track (track.index)}
					{@const levels = session.waves.tracks[track.index]}
					{#if levels}
						<WaveRow
							{levels}
							sliceSecs={session.waves.sliceSecs}
							{start}
							{span}
							{width}
							inSecs={session.inSecs}
							outSecs={session.outSecs}
							cuts={session.keptCuts}
							title={track.title}
							muted={session.effectiveVolume(track.index) === 0}
						/>
					{/if}
				{/each}
			{:else}
				<div class="flex h-7 items-center border-t border-line px-1 text-[11px] text-subtle" role="status">
					{session.wavesState === 'error' ? 'The waveforms could not be read.' : 'Reading the audio for waveforms'}
				</div>
			{/if}
		{/if}

		<!-- Stretches at another speed: a band along the bottom of the strip with the speed -->
		{#each speedBands as sp (sp.i)}
			<span
				class="speed"
				class:slow={sp.r.speed < 1}
				style:left="{sp.b.left}px"
				style:width="{sp.b.width}px"
				title="{speedLabel(sp.r.speed)} from {formatTimecode(sp.r.start)} to {formatTimecode(sp.r.end)}"
			>
				{#if sp.b.width > 34}<span class="tnum speed-label">{speedLabel(sp.r.speed)}</span>{/if}
			</span>
		{/each}

		<!-- The part Delete would take out, between cut points -->
		{#if segmentBand}
			<span class="segment" style:left="{segmentBand.left}px" style:width="{segmentBand.width}px" aria-hidden="true"></span>
		{/if}

		<!-- Cut-out parts and the selection, over the strip and the waveforms -->
		{#each cutBands as cut (cut.i)}
			<span class="cut" style:left="{cut.b.left}px" style:width="{cut.b.width}px" title="Cut out: {formatTimecode(cut.c.start)} to {formatTimecode(cut.c.end)}"></span>
		{/each}
		{#if selectionBand}
			<span class="selection" style:left="{selectionBand.left}px" style:width="{selectionBand.width}px" aria-hidden="true"></span>
		{/if}

		<!-- Bookmarks: moments marked with the clip hotkey while recording -->
		{#each session.clip.bookmarks as mark, i (i)}
			{@const x = xOf(mark)}
			{#if x >= -4 && x <= width + 4}
				<button
					type="button"
					class="mark"
					style:left="{x}px"
					title="Bookmark at {formatTimecode(mark)}"
					aria-label="Go to the bookmark at {formatTimecode(mark)}"
					onpointerdown={(e) => e.stopPropagation()}
					onclick={() => playback.seek(mark)}
				></button>
			{/if}
		{/each}

		<!-- Cut points, set with C: drag to move, Delete to take away -->
		{#each session.splits as at, i (i)}
			{@const x = xOf(at)}
			{#if x >= -4 && x <= width + 4}
				<div
					class="split"
					class:active={draggingSplit === i}
					style:left="{x}px"
					role="slider"
					tabindex="0"
					aria-label="Cut point"
					aria-orientation="horizontal"
					aria-valuemin={session.inSecs}
					aria-valuemax={session.outSecs}
					aria-valuenow={at}
					aria-valuetext={formatTimecode(at)}
					title="Cut point at {formatTimecode(at)}. Drag to move; Delete or C takes it away."
					onpointerdown={(e) => splitDown(e, i)}
					onpointermove={splitMove}
					onpointerup={splitUp}
					onpointercancel={splitUp}
					onkeydown={(e) => splitKeydown(e, i)}
				></div>
			{/if}
		{/each}

		<!-- Where a drag snapped to -->
		{#if snappedAt !== null}
			{@const x = xOf(snappedAt)}
			{#if x >= -1 && x <= width + 1}
				<span class="guide" style:left="{x}px" aria-hidden="true"></span>
			{/if}
		{/if}

		<!-- Playhead -->
		{#if playX >= -1 && playX <= width + 1}
			<span class="playhead" style:left="{playX}px" aria-hidden="true"></span>
		{/if}
	</div>
</div>
{/snippet}
</ContextMenu.Trigger>
<ContextMenu.Portal>
	<ContextMenu.Content
		class="z-50 min-w-56 origin-(--bits-context-menu-content-transform-origin) rounded-lg bg-popover p-1 text-popover-foreground shadow-md ring-1 ring-foreground/10 data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 duration-100"
	>
		<div class="flex items-center gap-2 px-2 pt-1 pb-1.5 text-xs text-muted-foreground">
			<GaugeIcon class="size-3.5" aria-hidden="true" />
			{#if menuTarget}
				Speed of <span class="tnum text-foreground">{formatTimecode(menuTarget.start)} to {formatTimecode(menuTarget.end)}</span>
			{:else}
				Select a part, or set cut points with C
			{/if}
		</div>
		<div class="grid grid-cols-4 gap-1 px-1 pb-1" role="group" aria-label="Speed">
			{#each SPEEDS as speed (speed)}
				<ContextMenu.Item
					class="tnum flex h-7 cursor-default items-center justify-center gap-1 rounded-md border border-line-control text-xs outline-hidden data-highlighted:bg-accent data-disabled:opacity-40 {Math.abs(menuSpeed - speed) < 1e-6 && menuTarget ? 'bg-on text-foreground' : 'text-muted-foreground'}"
					disabled={!menuTarget || session.busy}
					onSelect={() => menuTarget && session.setSpeed(menuTarget, speed)}
				>
					{#if Math.abs(menuSpeed - speed) < 1e-6 && menuTarget}<CheckIcon class="size-3" aria-hidden="true" />{/if}
					{speedLabel(speed)}
				</ContextMenu.Item>
			{/each}
		</div>
		<p class="px-2 pb-1 text-[11px] text-subtle">S slow motion, F fast, N normal.</p>
		<ContextMenu.Separator class="-mx-1 my-1 h-px bg-border" />
		<ContextMenu.Item class={menuItem} disabled={session.busy} onSelect={() => session.toggleCutAtPlayhead()}>
			<ScissorsIcon aria-hidden="true" /> Cut here
			<span class="ml-auto text-xs text-subtle">C</span>
		</ContextMenu.Item>
		<ContextMenu.Item
			class={menuItem}
			disabled={!menuTarget || session.busy}
			onSelect={() => (session.selection ? session.cutSelection() : session.deleteSegment())}
		>
			<span class="size-4" aria-hidden="true"></span> Take this part out
			<span class="ml-auto text-xs text-subtle">Del</span>
		</ContextMenu.Item>
		{#if session.selection}
			<ContextMenu.Item class={menuItem} disabled={session.busy} onSelect={() => session.keepSelection()}>
				<span class="size-4" aria-hidden="true"></span> Keep only this part
			</ContextMenu.Item>
		{/if}
	</ContextMenu.Content>
</ContextMenu.Portal>
</ContextMenu.Root>

<style>
	.tick {
		position: absolute;
		bottom: 0;
		width: 1px;
		height: 4px;
		background: var(--line-control);
		pointer-events: none;
	}
	.tick.major {
		height: 8px;
		background: var(--text-subtle);
	}
	.tick-label {
		position: absolute;
		top: 0;
		font-size: 12px;
		line-height: 16px;
		color: var(--text-subtle);
		white-space: nowrap;
		pointer-events: none;
	}
	.chip {
		position: absolute;
		top: 0;
		z-index: 4;
		padding: 0 4px;
		border-radius: 2px;
		background: #ededed;
		color: #0a0a0a;
		font-size: 12px;
		line-height: 16px;
		font-weight: 500;
		white-space: nowrap;
		pointer-events: none;
	}
	.chip:not(.chip-end) {
		transform: translateX(0);
	}
	.chip.chip-end {
		transform: translateX(-100%);
	}
	.dim {
		position: absolute;
		top: 0;
		bottom: 0;
		background: rgb(0 0 0 / 0.55);
		pointer-events: none;
	}
	.range {
		position: absolute;
		top: 0;
		bottom: 0;
		background: rgb(255 255 255 / 0.1);
		border-top: 1px solid var(--text-muted);
		border-bottom: 1px solid var(--text-muted);
		pointer-events: none;
	}
	.handle {
		position: absolute;
		top: 0;
		bottom: 0;
		z-index: 3;
		width: 8px;
		background: #ededed;
		cursor: ew-resize;
		outline: none;
		transition: background-color var(--dur-fast) var(--ease-out);
	}
	.handle::before {
		/* grip */
		content: '';
		position: absolute;
		top: 50%;
		left: 50%;
		width: 2px;
		height: 16px;
		transform: translate(-50%, -50%);
		background: #0a0a0a;
		opacity: 0.55;
		border-radius: 1px;
	}
	.handle::after {
		/* wider hit area */
		content: '';
		position: absolute;
		inset: 0 -6px;
	}
	.handle-in {
		transform: translateX(-100%);
		border-radius: 3px 0 0 3px;
	}
	.handle-out {
		border-radius: 0 3px 3px 0;
	}
	.handle:hover,
	.handle.active {
		background: #ffffff;
	}
	.handle:focus-visible {
		outline: 2px solid var(--focus);
		outline-offset: 1px;
	}
	.selection,
	.cut {
		position: absolute;
		top: 24px;
		bottom: 0;
		pointer-events: none;
	}
	.selection {
		z-index: 2;
		background: rgb(255 255 255 / 0.22);
		border-left: 1px solid #ffffff;
		border-right: 1px solid #ffffff;
	}
	.cut {
		z-index: 1;
		background: repeating-linear-gradient(
			-45deg,
			rgb(0 0 0 / 0.72) 0 6px,
			rgb(0 0 0 / 0.45) 6px 12px
		);
		border-left: 1px dashed var(--text-subtle);
		border-right: 1px dashed var(--text-subtle);
	}
	/* A flag at the top and a faint line down: bookmarks set while recording. */
	.mark {
		position: absolute;
		top: 0;
		bottom: 0;
		z-index: 4;
		width: 11px;
		margin-left: -5.5px;
		cursor: pointer;
		background: linear-gradient(var(--mark), var(--mark)) center / 1px 100% no-repeat;
		opacity: 0.75;
		transition: opacity var(--dur-fast) var(--ease-out);
	}
	.mark::before {
		content: '';
		position: absolute;
		top: 0;
		left: 50%;
		width: 9px;
		height: 9px;
		transform: translateX(-50%);
		background: var(--mark);
		clip-path: polygon(0 0, 100% 0, 100% 70%, 50% 100%, 0 70%);
	}
	.mark:hover,
	.mark:focus-visible {
		opacity: 1;
	}
	.mark:focus-visible {
		outline: 2px solid var(--focus);
		outline-offset: 1px;
	}
	/* A stretch at another speed: a band along the bottom of the strip. Slow motion is solid,
	   fast is striped, so the two read apart without colour. */
	.speed {
		position: absolute;
		top: 48px;
		height: 16px;
		z-index: 2;
		display: flex;
		align-items: center;
		justify-content: center;
		overflow: hidden;
		pointer-events: none;
		border-top: 1px solid rgb(255 255 255 / 0.55);
		background: repeating-linear-gradient(90deg, rgb(255 255 255 / 0.16) 0 6px, rgb(255 255 255 / 0.06) 6px 9px);
	}
	.speed.slow {
		background: rgb(255 255 255 / 0.14);
	}
	.speed-label {
		padding: 0 4px;
		font-size: 11px;
		line-height: 14px;
		font-weight: 500;
		color: var(--text);
		background: rgb(0 0 0 / 0.55);
		border-radius: 2px;
	}
	/* The part between cut points that holds the playhead: what Delete takes out. */
	.segment {
		position: absolute;
		top: 24px;
		height: 40px;
		z-index: 1;
		pointer-events: none;
		background: rgb(255 255 255 / 0.09);
		box-shadow: inset 0 0 0 1.5px rgb(255 255 255 / 0.7);
		border-radius: 2px;
	}
	/* A cut point: a line through the strip with a diamond on the ruler, the edit point of
	   most editors. Wide hit area, thin mark. */
	.split {
		position: absolute;
		top: 12px;
		bottom: 0;
		z-index: 4;
		width: 13px;
		margin-left: -6.5px;
		cursor: ew-resize;
		outline: none;
		background: linear-gradient(var(--text), var(--text)) center 12px / 2px calc(100% - 12px) no-repeat;
		opacity: 0.8;
		transition: opacity var(--dur-fast) var(--ease-out);
	}
	.split::before {
		content: '';
		position: absolute;
		top: 1px;
		left: 50%;
		width: 9px;
		height: 9px;
		transform: translateX(-50%) rotate(45deg);
		border-radius: 1px;
		background: var(--text);
	}
	.split:hover,
	.split.active {
		opacity: 1;
	}
	.split:focus-visible {
		opacity: 1;
		outline: 2px solid var(--focus);
		outline-offset: 1px;
	}
	.guide {
		position: absolute;
		top: 0;
		bottom: 0;
		z-index: 6;
		width: 0;
		margin-left: -0.5px;
		border-left: 1px dashed var(--text);
		pointer-events: none;
	}
	.playhead {
		position: absolute;
		top: 0;
		bottom: 0;
		z-index: 5;
		width: 1px;
		margin-left: -0.5px;
		background: #ffffff;
		pointer-events: none;
	}
	.playhead::before {
		content: '';
		position: absolute;
		top: 0;
		left: 50%;
		width: 9px;
		height: 8px;
		transform: translateX(-50%);
		background: #ffffff;
		clip-path: polygon(0 0, 100% 0, 50% 100%);
	}
</style>
