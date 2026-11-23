<script lang="ts">
	import PauseIcon from '@lucide/svelte/icons/pause';
	import PlayIcon from '@lucide/svelte/icons/play';
	import SquareIcon from '@lucide/svelte/icons/square';
	import type { FileRecordingStatus } from '$lib/api/types';
	import { app, pauseFileRecording, stopFileRecording } from '$lib/stores/app.svelte';
	import { formatBytes, formatDuration } from '$lib/format/format';
	import SourcePreview from '../SourcePreview.svelte';

	/**
	 * The recording that is running, at the head of the library: a live still of what it
	 * records, how long it has run and how big it is, and a way to stop it.
	 */
	let { file }: { file: FileRecordingStatus } = $props();

	const session = $derived(file.kind === 'session');
</script>

<div class="live flex min-w-0 flex-col overflow-hidden rounded-md border bg-chrome" role="group" aria-label="{session ? 'Session' : 'Recording'} in progress">
	<span class="relative block aspect-video w-full overflow-hidden bg-black">
		<SourcePreview target={file.source} maxWidth={360} fit="contain" empty="" class="absolute inset-0 bg-black" />
		<span class="scrim absolute inset-0" aria-hidden="true"></span>
		<span class="absolute top-2 left-2 flex items-center gap-1.5 rounded-xs bg-black/80 px-1.5 py-0.5 text-xs font-medium text-foreground">
			<span class="dot" class:still={file.paused} aria-hidden="true"></span>
			{file.paused ? 'Paused' : session ? 'Recording this session' : 'Recording'}
		</span>
		<span class="tnum absolute right-1 bottom-1 rounded-xs bg-black/80 px-1 text-xs font-medium text-foreground" aria-label="Recorded so far {formatDuration(file.secs)}">
			{formatDuration(file.secs)}
		</span>
	</span>
	<span class="flex min-w-0 items-center gap-2 border-t border-line px-2.5 pt-2 pb-2.5">
		<span class="flex min-w-0 flex-1 flex-col gap-0.5">
			<span class="truncate text-sm font-medium text-foreground">{session ? `${file.game} session` : `Recording ${file.game}`}</span>
			<span class="tnum truncate text-xs text-muted-foreground">
				{formatBytes(file.bytes)} so far{session ? ' · stops when the game closes' : ''}
			</span>
		</span>
		<button
			type="button"
			class="pause"
			onclick={() => pauseFileRecording(!file.paused)}
			disabled={app.stoppingFile}
			aria-pressed={file.paused}
			aria-label={file.paused ? 'Resume recording' : 'Pause recording'}
			title={file.paused ? 'Resume: carry on recording' : 'Pause: leave out what happens until you resume'}
		>
			{#if file.paused}
				<PlayIcon class="size-3 fill-current" aria-hidden="true" />
			{:else}
				<PauseIcon class="size-3 fill-current" aria-hidden="true" />
			{/if}
		</button>
		<button type="button" class="stop" onclick={stopFileRecording} disabled={app.stoppingFile} title="Stop and save the recording">
			<SquareIcon class="size-3 fill-current" aria-hidden="true" />
			Stop
		</button>
	</span>
</div>

<style>
	.live {
		border-color: color-mix(in srgb, var(--danger) 45%, transparent);
	}
	/* A faint red wash says "live" without hiding the picture. */
	.scrim {
		background: linear-gradient(to bottom, rgb(220 38 38 / 0.16), transparent 45%);
	}
	.dot {
		width: 8px;
		height: 8px;
		border-radius: 999px;
		background: var(--danger-solid);
		animation: pulse 1.6s ease-in-out infinite;
	}
	@keyframes pulse {
		50% {
			opacity: 0.4;
		}
	}
	.dot.still {
		animation: none;
		opacity: 0.5;
	}
	.pause,
	.stop {
		display: inline-flex;
		flex-shrink: 0;
		align-items: center;
		gap: 6px;
		height: 28px;
		padding: 0 10px;
		border-radius: 6px;
		border: 1px solid color-mix(in srgb, var(--danger) 55%, transparent);
		background: var(--danger-tint);
		font-size: 13px;
		font-weight: 500;
		color: var(--text);
		transition:
			background-color var(--dur-fast) var(--ease-out),
			transform var(--dur-fast) var(--ease-out);
	}
	.stop:hover:not(:disabled) {
		background: color-mix(in srgb, var(--danger) 22%, transparent);
	}
	.pause {
		justify-content: center;
		width: 28px;
		padding: 0;
		border-color: var(--line-control);
		background: transparent;
	}
	.pause:hover:not(:disabled) {
		background: var(--surface-raised);
	}
	.pause:active:not(:disabled),
	.stop:active:not(:disabled) {
		transform: scale(0.97);
	}
	.pause:focus-visible,
	.stop:focus-visible {
		outline: 2px solid var(--focus);
		outline-offset: 2px;
	}
	@media (prefers-reduced-motion: reduce) {
		.dot {
			animation: none;
		}
	}
</style>
