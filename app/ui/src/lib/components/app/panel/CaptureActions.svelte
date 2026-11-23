<script lang="ts">
	import CameraIcon from '@lucide/svelte/icons/camera';
	import ClapperboardIcon from '@lucide/svelte/icons/clapperboard';
	import PauseIcon from '@lucide/svelte/icons/pause';
	import PlayIcon from '@lucide/svelte/icons/play';
	import RotateCcwIcon from '@lucide/svelte/icons/rotate-ccw';
	import { Button } from '$lib/components/ui/button';
	import { Spinner } from '$lib/components/ui/spinner';
	import {
		app,
		pauseFileRecording,
		saveClipNow,
		startFileRecording,
		stopFileRecording,
		takeScreenshotNow
	} from '$lib/stores/app.svelte';
	import { comboLabels } from '$lib/format/hotkey';
	import { formatBytes, formatDuration } from '$lib/format/format';
	import type { CaptureTarget } from '$lib/api/types';
	import RecordSourceDialog from './RecordSourceDialog.svelte';

	const recording = $derived(app.status?.state === 'recording');
	const capacity = $derived(app.settings?.clip.lengthSecs ?? 0);
	const clipKeys = $derived(app.settings ? comboLabels(app.settings.clip.hotkey) : []);
	const shotKeys = $derived(app.settings ? comboLabels(app.settings.screenshot.hotkey) : []);
	const recordKeys = $derived(app.settings ? comboLabels(app.settings.recording.recordHotkey) : []);
	const pauseKeys = $derived(app.settings ? comboLabels(app.settings.recording.pauseHotkey) : []);
	const file = $derived(app.status?.file ?? null);
	let starting = $state(false);

	// The start shows as busy until the status reports the recording.
	$effect(() => {
		if (file) starting = false;
	});

	/** Record asks what to record first; the hotkey records the capture straight away. */
	let choosing = $state(false);

	async function toggleRecord() {
		if (file) await stopFileRecording();
		else choosing = true;
	}

	async function record(target: CaptureTarget) {
		starting = true;
		await startFileRecording('manual', target);
		// Recording may take a moment to start when the replay buffer was off.
		setTimeout(() => (starting = false), 4000);
	}
</script>

{#snippet hint(keys: string[], label: string)}
	{#if keys.length}
		<!-- The hotkey is part of the button: a quiet second line, read out with the label. -->
		<span class="hint tnum max-w-full truncate text-xs font-normal" aria-label="{label}: {keys.join(' plus ')}" title={keys.join(' + ')}>
			{keys.join(' + ')}
		</span>
	{/if}
{/snippet}

<section class="grid grid-cols-2 gap-2 border-b border-line px-3 py-3" aria-label="Save and record">
	<Button
		class="action h-auto flex-col gap-0.5 py-2"
		onclick={saveClipNow}
		disabled={!recording || app.saving}
		title={recording ? `Save the last ${capacity} seconds` : 'Start recording to save clips'}
	>
		<span class="flex items-center gap-1.5">
			{#if app.saving}<Spinner />{:else}<ClapperboardIcon aria-hidden="true" />{/if}
			Save clip
		</span>
		{@render hint(clipKeys, 'Hotkey')}
	</Button>
	<Button
		variant="outline"
		class="action h-auto flex-col gap-0.5 py-2"
		onclick={takeScreenshotNow}
		disabled={app.shooting}
		title="Save a still of what is being captured"
	>
		<span class="flex items-center gap-1.5">
			{#if app.shooting}<Spinner />{:else}<CameraIcon aria-hidden="true" />{/if}
			Screenshot
		</span>
		{@render hint(shotKeys, 'Hotkey')}
	</Button>

	<div class="col-span-2 flex gap-2">
		<button
			type="button"
			class="record flex h-9 min-w-0 flex-1 items-center gap-2 rounded-lg border px-3 text-sm font-medium"
			class:live={!!file}
			class:paused={!!file?.paused}
			onclick={toggleRecord}
			disabled={app.stoppingFile || (starting && !file)}
			aria-pressed={!!file}
			title={file
				? 'Stop and save the recording'
				: 'Record what is being captured until you stop it. Clips keep working meanwhile.'}
		>
			{#if app.stoppingFile || (starting && !file)}
				<Spinner class="size-3.5" />
			{:else}
				<span class="dot" aria-hidden="true"></span>
			{/if}
			{#if file}
				<span>Stop</span>
				<span class="tnum ml-auto truncate text-xs font-normal" aria-label="Recorded {formatDuration(file.secs)}, {formatBytes(file.bytes)}">
					{formatDuration(file.secs)} · {formatBytes(file.bytes)}
				</span>
			{:else}
				<span>{starting ? 'Starting' : 'Record'}</span>
				{#if recordKeys.length}
					<span class="hint tnum ml-auto text-xs font-normal" aria-label="Hotkey: {recordKeys.join(' plus ')}">{recordKeys.join(' + ')}</span>
				{/if}
			{/if}
		</button>
		{#if file}
			<Button
				variant="outline"
				size="icon"
				class="size-9 shrink-0 rounded-lg"
				onclick={() => pauseFileRecording(!file.paused)}
				disabled={app.stoppingFile}
				aria-pressed={file.paused}
				aria-label={file.paused ? 'Resume recording' : 'Pause recording'}
				title="{file.paused ? 'Resume: carry on recording' : 'Pause: leave out what happens until you resume'}{pauseKeys.length ? ` (${pauseKeys.join(' + ')})` : ''}"
			>
				{#if file.paused}<PlayIcon />{:else}<PauseIcon />{/if}
			</Button>
		{/if}
	</div>
	{#if !file && app.status?.game && (app.status.sessionStopped || !app.settings?.recording.fullSessions)}
		<!-- A game is running without its session being recorded: offer to (re)start it. -->
		<div class="session col-span-2 -mt-1 flex flex-col gap-2 rounded-lg border border-line p-2.5">
			<span class="text-xs text-muted-foreground">
				{#if app.status.sessionStopped}
					You stopped the <span class="text-foreground">{app.status.game}</span> session.
				{:else}
					<span class="text-foreground">{app.status.game}</span> is running.
				{/if}
			</span>
			<Button
				variant="outline"
				size="sm"
				class="w-full"
				onclick={() => startFileRecording('session')}
				title="Record {app.status.game} from now until it closes"
			>
				<RotateCcwIcon data-icon="inline-start" />
				{app.status.sessionStopped ? 'Restart session' : 'Record session'}
			</Button>
		</div>
	{/if}
	{#if file}
		<p class="col-span-2 -mt-1 text-xs text-muted-foreground" role="status">
			{#if file.paused}
				Paused. Nothing is recorded until you resume, and the file carries on without a gap.
			{:else}
				{file.kind === 'session' ? `Recording the ${file.game} session; it stops when the game closes.` : `Recording ${file.game}.`}
				The clip hotkey still saves clips and marks the moment.
			{/if}
		</p>
	{/if}
</section>

<RecordSourceDialog bind:open={choosing} onrecord={record} />

<style>
	/* On the light button the hint is the button's own ink, faded; on the outline one, muted. */
	:global(.action) .hint,
	.record .hint {
		opacity: 0.62;
		letter-spacing: 0.01em;
	}
	.record {
		border-color: var(--line-control);
		color: var(--text);
		transition:
			background-color var(--dur-fast) var(--ease-out),
			border-color var(--dur-fast) var(--ease-out),
			transform var(--dur-fast) var(--ease-out);
	}
	.record:hover:not(:disabled) {
		background: var(--surface-raised);
	}
	.record:active:not(:disabled) {
		transform: scale(0.98);
	}
	.record:disabled {
		opacity: 0.6;
		cursor: progress;
	}
	.record:focus-visible {
		outline: 2px solid var(--focus);
		outline-offset: 2px;
	}
	.record.live {
		border-color: color-mix(in srgb, var(--danger) 55%, transparent);
		background: var(--danger-tint);
	}
	.record.live:hover:not(:disabled) {
		background: color-mix(in srgb, var(--danger) 20%, transparent);
	}
	.dot {
		width: 10px;
		height: 10px;
		flex-shrink: 0;
		border-radius: 999px;
		background: var(--danger-solid);
	}
	.record.live .dot {
		border-radius: 2px;
		animation: pulse 1.6s ease-in-out infinite;
	}
	/* Paused: the square stops pulsing and dims, the time stands still. */
	.record.live.paused .dot {
		animation: none;
		opacity: 0.5;
	}
	@keyframes pulse {
		50% {
			opacity: 0.45;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.record.live .dot {
			animation: none;
		}
	}
</style>
