<script lang="ts">
	import PlayIcon from '@lucide/svelte/icons/play';
	import SquareIcon from '@lucide/svelte/icons/square';
	import { Button } from '$lib/components/ui/button';
	import { app, toggleRecording } from '$lib/stores/app.svelte';
	import { EXPLAIN } from '$lib/format/explain';
	import { formatBytes, formatDuration, formatFps, formatResolution } from '$lib/format/format';
	import Term from '../Term.svelte';

	const status = $derived(app.status);
	const recording = $derived(status?.state === 'recording');
	const errored = $derived(status?.state === 'error');
	const capacity = $derived(app.settings?.clip.lengthSecs ?? 0);
	const fill = $derived(capacity > 0 && status ? Math.min(100, Math.round((status.bufferSecs / capacity) * 100)) : 0);
	// "Recording" is for recordings to a file; this is the replay buffer clips come from.
	const stateLabel = $derived(recording ? 'Replay buffer on' : errored ? 'Error' : 'Replay buffer off');
	/** A recording to a file needs the buffer, so it can't be turned off meanwhile. */
	const fileRunning = $derived(!!status?.file);
	/** Tracks recorded as silence; the microphone's own section shows its problem. */
	const audioProblems = $derived((status?.audioProblems ?? []).filter((p) => p.track !== 'Microphone'));
</script>

<section class="flex flex-col gap-3 border-b border-line px-3 py-3" aria-label="Recording status">
	<div class="flex items-center gap-2">
		<span class="dot" class:recording class:errored aria-hidden="true"></span>
		<span class="flex-1 text-sm font-medium text-foreground" role="status">{stateLabel}</span>
		<Button
			variant="outline"
			size="sm"
			onclick={toggleRecording}
			disabled={app.togglingRecording || !status || fileRunning}
			title={fileRunning
				? 'The recording below needs the replay buffer; stop it first'
				: recording
					? 'Turn the replay buffer off and clear it'
					: 'Turn the replay buffer on, so clips can be saved'}
		>
			{#if recording}
				<SquareIcon data-icon="inline-start" class="size-3 fill-current" />
				Turn off
			{:else}
				<PlayIcon data-icon="inline-start" class="size-3 fill-current" />
				Turn on
			{/if}
		</Button>
	</div>

	{#if status?.game && !errored}
		<!-- Which game Umbra sees, and why it counts as one. -->
		<p class="flex min-w-0 items-baseline gap-1.5 text-xs" title="Counts as a game: {status.gameReason ?? ''}">
			<span class="truncate text-foreground">{status.game}</span>
			{#if status.gameReason}<span class="shrink-0 truncate text-subtle">· {status.gameReason}</span>{/if}
		</p>
	{/if}
	{#if errored && status}
		<p class="selectable text-xs text-danger" role="alert">{status.error ?? 'The recorder stopped unexpectedly.'}</p>
	{:else if recording && status}
		{#if status.target}
			<p class="truncate text-xs text-muted-foreground" title={status.target}>
				{status.game ? status.target : `Desktop · ${status.target}`}
			</p>
		{/if}
		<dl class="grid grid-cols-[auto_1fr] items-center gap-x-3 gap-y-2 text-xs">
			<dt class="text-muted-foreground"><Term label={status.bufferOnDisk ? 'Buffer on disk' : 'Buffer'} explain={EXPLAIN.buffer} /></dt>
			<dd class="tnum text-right text-foreground">{formatDuration(status.bufferSecs)} / {formatDuration(capacity)}</dd>
			<dd class="col-span-2 flex items-center gap-2">
				<span class="relative h-1 flex-1 overflow-hidden rounded-xs bg-selected" role="meter" aria-label="Buffer fill" aria-valuemin={0} aria-valuemax={100} aria-valuenow={fill}>
					<span class="absolute inset-0 origin-left bg-muted-foreground" style:transform="scaleX({fill / 100})" style:transition="transform 1s linear"></span>
				</span>
				<span class="tnum w-14 text-right text-foreground" title={status.bufferOnDisk ? 'On disk' : 'In memory'}>{formatBytes(status.bufferBytes)}</span>
			</dd>
			<dt class="text-muted-foreground">Output</dt>
			<dd class="tnum truncate text-right text-foreground" title={status.hdr === 'hdr10' ? 'The screen is in HDR; recorded as HDR10' : status.hdr === 'sdr' ? 'The screen is in HDR; converted to SDR' : ''}>
				{formatResolution(status.width, status.height)} &middot; {formatFps(status.fps)} fps{status.hdr === 'hdr10' ? ' · HDR10' : status.hdr === 'sdr' ? ' · HDR→SDR' : ''}
			</dd>
			<dt class="text-muted-foreground"><Term label="Dropped frames" explain={EXPLAIN.droppedFrames} /></dt>
			<dd class="tnum text-right {status.droppedFrames > 0 ? 'font-semibold text-foreground' : 'text-foreground'}">{status.droppedFrames}</dd>
		</dl>
		{#each audioProblems as p (p.track)}
			<p class="selectable text-xs text-danger" role="alert">{p.track} is silent: {p.problem}</p>
		{/each}
	{:else}
		<p class="text-xs text-muted-foreground">Nothing is kept in memory, so clips can't be saved. Record still works.</p>
	{/if}
</section>

<style>
	.dot {
		width: 8px;
		height: 8px;
		flex-shrink: 0;
		border-radius: 999px;
		border: 1.5px solid var(--text-subtle);
		transition:
			background-color var(--dur-base) var(--ease-out),
			border-color var(--dur-base) var(--ease-out);
	}
	.dot.recording {
		background: var(--success-solid);
		border-color: var(--success-solid);
	}
	.dot.errored {
		background: var(--danger-solid);
		border-color: var(--danger-solid);
	}
</style>
