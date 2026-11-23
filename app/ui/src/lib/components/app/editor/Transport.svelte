<script lang="ts">
	import ChevronLeftIcon from '@lucide/svelte/icons/chevron-left';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import PlayIcon from '@lucide/svelte/icons/play';
	import PauseIcon from '@lucide/svelte/icons/pause';
	import SkipBackIcon from '@lucide/svelte/icons/skip-back';
	import SkipForwardIcon from '@lucide/svelte/icons/skip-forward';
	import RepeatIcon from '@lucide/svelte/icons/repeat';
	import Volume2Icon from '@lucide/svelte/icons/volume-2';
	import VolumeXIcon from '@lucide/svelte/icons/volume-x';
	import ZoomInIcon from '@lucide/svelte/icons/zoom-in';
	import ZoomOutIcon from '@lucide/svelte/icons/zoom-out';
	import KeyboardIcon from '@lucide/svelte/icons/keyboard';
	import { Button } from '$lib/components/ui/button';
	import { Slider } from '$lib/components/ui/slider';
	import * as Popover from '$lib/components/ui/popover';
	import { Kbd } from '$lib/components/ui/kbd';
	import IconButton from '$lib/components/app/IconButton.svelte';
	import type { EditSession } from '$lib/editor/session.svelte';
	import { formatTimecode } from '$lib/format/format';
	import { editorShortcuts } from '$lib/format/shortcuts';
	import ShortcutList from '$lib/components/app/ShortcutList.svelte';
	import ScissorsIcon from '@lucide/svelte/icons/scissors';
	import XIcon from '@lucide/svelte/icons/x';
	import MagnetIcon from '@lucide/svelte/icons/magnet';
	import Undo2Icon from '@lucide/svelte/icons/undo-2';
	import Redo2Icon from '@lucide/svelte/icons/redo-2';

	let { session }: { session: EditSession } = $props();

	const playback = $derived(session.playback);

	const shortcuts = $derived(editorShortcuts(session.seekSecs));
	const selection = $derived(session.selection);

	const speedLabel = $derived(
		playback.playing && Math.abs(playback.speed) !== 1 ? `${playback.speed < 0 ? '−' : ''}${Math.abs(playback.speed)}×` : ''
	);
</script>

<div class="flex h-10 shrink-0 items-center gap-2 px-3" role="toolbar" aria-label="Playback controls">
	<IconButton label="Go to trim start" shortcut="Home" icon={SkipBackIcon} onclick={() => playback.seek(session.inSecs)} />
	<IconButton label="Previous frame" shortcut="," icon={ChevronLeftIcon} onclick={() => playback.step(-1)} />
	<IconButton
		label={playback.playing ? 'Pause' : 'Play'}
		shortcut="Space"
		icon={playback.playing ? PauseIcon : PlayIcon}
		variant="secondary"
		onclick={() => playback.toggle()}
	/>
	<IconButton label="Next frame" shortcut="." icon={ChevronRightIcon} onclick={() => playback.step(1)} />
	<IconButton label="Go to trim end" shortcut="End" icon={SkipForwardIcon} onclick={() => playback.seek(session.outSecs)} />
	<IconButton
		label="Loop the trimmed range"
		icon={RepeatIcon}
		pressed={playback.loop}
		onclick={() => (playback.loop = !playback.loop)}
	/>

	<div class="mx-2 flex items-baseline gap-2 whitespace-nowrap" aria-live="off">
		<span class="tnum min-w-[72px] text-base font-medium text-foreground" aria-label="Playhead {formatTimecode(playback.time)}">
			{formatTimecode(playback.time)}
		</span>
		<span class="tnum text-xs text-subtle @max-[700px]:hidden">/ {formatTimecode(playback.duration)}</span>
		<span class="tnum min-w-[72px] text-xs text-subtle @max-[1000px]:hidden">Frame {playback.frame.toLocaleString('en-US')}</span>
		<span class="tnum min-w-6 text-xs text-foreground">{speedLabel}</span>
	</div>

	{#if selection}
		<!-- A selection swaps the trim buttons for what can be done with it. -->
		<div class="flex items-center gap-2" role="group" aria-label="Selection">
			<span class="tnum text-xs whitespace-nowrap text-foreground">
				<span class="@max-[920px]:hidden">Selected</span> {(selection.end - selection.start).toFixed(2)} s
			</span>
			<Button variant="outline" size="sm" onclick={() => session.cutSelection()} disabled={session.busy} aria-keyshortcuts="Delete" title="Leave the selected part out of the export (Delete)">
				<ScissorsIcon data-icon="inline-start" />
				Cut out <Kbd>Del</Kbd>
			</Button>
			<Button variant="outline" size="sm" onclick={() => session.keepSelection()} disabled={session.busy} title="Trim the clip to the selection">
				Keep<span class="@max-[920px]:hidden">&nbsp;only this</span>
			</Button>
			<IconButton label="Clear the selection" shortcut="Esc" icon={XIcon} size="icon-sm" onclick={() => session.clearSelection()} />
		</div>
	{:else}
		<Button variant="outline" size="sm" onclick={() => session.setInAtPlayhead()} disabled={session.busy} aria-keyshortcuts="I" title="Set the start of the trim at the playhead">
			Set start <Kbd>I</Kbd>
		</Button>
		<Button variant="outline" size="sm" onclick={() => session.setOutAtPlayhead()} disabled={session.busy} aria-keyshortcuts="O" title="Set the end of the trim at the playhead">
			Set end <Kbd>O</Kbd>
		</Button>
		<Button
			variant="outline"
			size="sm"
			onclick={() => session.toggleCutAtPlayhead()}
			disabled={session.busy}
			aria-keyshortcuts="C"
			title="Cut at the playhead, or take away the cut point there. Then Delete takes out the part you are in; Q and W everything before or after it."
		>
			<ScissorsIcon data-icon="inline-start" />
			Cut <Kbd>C</Kbd>
		</Button>
	{/if}
	<IconButton label="Undo" shortcut="Ctrl+Z" icon={Undo2Icon} size="icon-sm" onclick={() => session.undo()} disabled={session.undoDepth === 0 || session.busy} />
	<IconButton label="Redo" shortcut="Ctrl+Y" icon={Redo2Icon} size="icon-sm" onclick={() => session.redo()} disabled={session.redoDepth === 0 || session.busy} />

	<div class="ml-auto flex items-center gap-1">
		<IconButton
			label={playback.muted ? 'Unmute preview' : 'Mute preview'}
			icon={playback.muted ? VolumeXIcon : Volume2Icon}
			onclick={() => playback.setMuted(!playback.muted)}
		/>
		<div class="w-20 px-1 @max-[920px]:hidden">
			<Slider
				type="single"
				min={0}
				max={1}
				step={0.01}
				value={playback.muted ? 0 : playback.volume}
				onValueChange={(v) => {
					playback.setVolume(v);
					if (playback.muted && v > 0) playback.setMuted(false);
				}}
				aria-label="Preview volume"
			/>
		</div>
		<span class="mx-1 h-4 w-px bg-line"></span>
		<IconButton
			label={session.magnet ? 'Snapping on: drags stick to cut points, bookmarks and trim ends (hold Alt to drag freely)' : 'Snapping off'}
			icon={MagnetIcon}
			pressed={session.magnet}
			onclick={() => session.toggleMagnet()}
		/>
		<IconButton label="Zoom timeline out" icon={ZoomOutIcon} onclick={() => session.zoomAt(0.6, playback.time)} disabled={!session.zoomed} />
		<IconButton label="Zoom timeline in" icon={ZoomInIcon} onclick={() => session.zoomAt(1.6, playback.time)} />
		<Button variant="ghost" size="sm" class="@max-[820px]:hidden" onclick={() => session.fitView()} disabled={!session.zoomed} title="Show the whole clip">Fit</Button>
		<Popover.Root>
			<Popover.Trigger>
				{#snippet child({ props })}
					<Button {...props} variant="ghost" size="icon-sm" aria-label="Keyboard shortcuts" title="Keyboard shortcuts">
						<KeyboardIcon />
					</Button>
				{/snippet}
			</Popover.Trigger>
			<Popover.Content align="end" side="top" class="w-96">
				<Popover.Header>
					<Popover.Title>Keyboard shortcuts</Popover.Title>
				</Popover.Header>
				<ShortcutList items={shortcuts} dense />
			</Popover.Content>
		</Popover.Root>
	</div>
</div>
