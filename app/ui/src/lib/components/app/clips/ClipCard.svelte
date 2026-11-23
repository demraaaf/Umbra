<script lang="ts">
	import { dragOut } from '$lib/actions/dragOut';
	import CheckIcon from '@lucide/svelte/icons/check';
	import ImageIcon from '@lucide/svelte/icons/image';
	import HeartIcon from '@lucide/svelte/icons/heart';
	import type { Clip } from '$lib/api/types';
	import LayerBadges from './LayerBadges.svelte';
	import { isFavorite, toggleFavorite } from '$lib/stores/app.svelte';
	import {
		clipTitle,
		formatBytes,
		formatDuration,
		formatFormat,
		formatFullDate,
		formatResolution,
		formatWhen,
		gameLabel,
		recordingKind
	} from '$lib/format/format';

	let {
		clip,
		selected = false,
		selecting = false,
		onactivate,
		ontoggle
	}: {
		clip: Clip;
		selected?: boolean;
		/** Something is selected, so the select boxes stay visible. */
		selecting?: boolean;
		onactivate: (clip: Clip, event: MouseEvent) => void;
		ontoggle: (clip: Clip) => void;
	} = $props();

	const image = $derived(clip.kind === 'image');
	const favorite = $derived(isFavorite(clip));
	const recorded = $derived(recordingKind(clip));

	// ---------- Hover preview ----------
	// Resting on a clip plays it, muted, inside the thumbnail. A short delay keeps a pointer
	// that only passes over the grid from starting a video on every card.

	const PREVIEW_DELAY_MS = 350;
	const canPreview = $derived(!image && !clip.videoUrl.startsWith('mock:'));
	let previewing = $state(false);
	let playing = $state(false);
	let progress = $state(0);
	let timer: ReturnType<typeof setTimeout> | null = null;

	function enter(e: PointerEvent) {
		// Not while a button is held: the middle button sweeps a selection over the cards.
		if (!canPreview || e.buttons !== 0 || e.pointerType === 'touch') return;
		timer = setTimeout(() => (previewing = true), PREVIEW_DELAY_MS);
	}
	function leave() {
		if (timer) clearTimeout(timer);
		timer = null;
		previewing = false;
		playing = false;
		progress = 0;
	}
	$effect(() => leave);
</script>

<div class="clip-card group relative" class:selected class:selecting data-clip-path={clip.path}>
	<button
		type="button"
		class="card-main flex w-full min-w-0 flex-col overflow-hidden rounded-md border border-line bg-chrome text-left outline-none"
		data-clip-card
		title={clip.fileName}
		use:dragOut={clip.path}
		onclick={(e) => onactivate(clip, e)}
		onpointerenter={enter}
		onpointerleave={leave}
	>
		<span class="thumb relative block aspect-video w-full overflow-hidden bg-raised">
			{#if clip.thumbnailUrl}
				<img src={clip.thumbnailUrl} alt="" class="size-full object-cover" loading="lazy" decoding="async" draggable="false" />
			{/if}
			{#if previewing}
				<!-- svelte-ignore a11y_media_has_caption -->
				<video
					src={clip.videoUrl}
					class="preview absolute inset-0 size-full object-cover"
					class:playing
					muted
					autoplay
					loop
					playsinline
					disablepictureinpicture
					disableremoteplayback
					preload="auto"
					aria-hidden="true"
					onplaying={() => (playing = true)}
					ontimeupdate={(e) => {
						const v = e.currentTarget;
						progress = v.duration > 0 ? v.currentTime / v.duration : 0;
					}}
				></video>
				<span class="preview-progress absolute inset-x-0 bottom-0 h-0.5 origin-left bg-foreground" style:transform="scaleX({progress})" aria-hidden="true"></span>
			{/if}
			{#if image}
				<span class="absolute right-1 bottom-1 flex items-center gap-1 rounded-xs bg-black/80 px-1 text-xs font-medium text-foreground">
					<ImageIcon class="size-3" aria-hidden="true" />
					Screenshot
				</span>
			{:else}
				<LayerBadges {clip} class="absolute bottom-1 left-1 rounded-xs bg-black/80 px-1 py-0.5 text-foreground" />
				<span class="absolute right-1 bottom-1 flex items-center gap-1">
					{#if recorded}
						<span class="flex items-center gap-1 rounded-xs bg-black/80 px-1 text-xs font-medium text-foreground">
							<span class="size-1.5 rounded-full bg-[var(--danger-solid)]" aria-hidden="true"></span>
							{recorded === 'session' ? 'Session' : 'Recording'}
						</span>
					{/if}
					<span class="tnum rounded-xs bg-black/80 px-1 text-xs font-medium text-foreground" aria-label="Length {formatDuration(clip.durationSecs)}">
						{formatDuration(clip.durationSecs)}
					</span>
				</span>
			{/if}
		</span>

		<span class="info flex min-w-0 flex-col gap-0.5 border-t border-line px-2.5 pt-2 pb-2.5">
			<!-- Room on the right for the favorite heart, which sits over this line. -->
			<span class="truncate pr-7 text-sm font-medium text-foreground">{clipTitle(clip)}</span>
			<span class="flex min-w-0 items-center gap-1 text-xs text-muted-foreground">
				<span class="truncate">{gameLabel(clip.game)}</span>
				<span aria-hidden="true">&middot;</span>
				<time class="shrink-0" datetime={clip.createdAt} title={formatFullDate(clip.createdAt)}>{formatWhen(clip.createdAt)}</time>
			</span>
			<span class="tnum truncate text-xs text-subtle">
				{image ? formatResolution(clip.width, clip.height) : formatFormat(clip)} &middot; {formatBytes(clip.sizeBytes)}
			</span>
		</span>
	</button>

	<button
		type="button"
		class="heart"
		class:on={favorite}
		aria-pressed={favorite}
		aria-label={favorite ? `Remove ${clipTitle(clip)} from Favorites` : `Add ${clipTitle(clip)} to Favorites`}
		title={favorite ? 'Remove from Favorites' : 'Add to Favorites'}
		onclick={() => toggleFavorite(clip.path)}
	>
		<HeartIcon class="size-4" strokeWidth={favorite ? 0 : 2} fill={favorite ? 'currentColor' : 'none'} aria-hidden="true" />
	</button>

	<button
		type="button"
		class="select-box"
		role="checkbox"
		aria-checked={selected}
		aria-label="Select {clipTitle(clip)}"
		onclick={() => ontoggle(clip)}
	>
		{#if selected}<CheckIcon class="size-3" strokeWidth={3} aria-hidden="true" />{/if}
	</button>
</div>

<style>
	.clip-card {
		/* The heart is placed against the thumbnail's height, which is 9/16 of the width. */
		container-type: inline-size;
	}
	.card-main {
		transition:
			border-color var(--dur-base) var(--ease-out),
			transform var(--dur-fast) var(--ease-out);
	}
	.clip-card:hover .card-main {
		border-color: var(--text-subtle);
	}
	.clip-card.selected .card-main {
		border-color: var(--text);
		box-shadow: 0 0 0 1px var(--text);
	}
	.card-main:focus-visible {
		outline: 2px solid var(--focus);
		outline-offset: 2px;
	}
	.card-main:active {
		transform: scale(0.99);
	}
	.preview {
		opacity: 0;
		transition: opacity 160ms var(--ease-out);
	}
	.preview.playing {
		opacity: 1;
	}
	.heart {
		position: absolute;
		/* On the name line, just under the 16:9 thumbnail (1px card border included). */
		right: 5px;
		top: calc(56.25cqw + 5px);
		display: flex;
		width: 28px;
		height: 28px;
		align-items: center;
		justify-content: center;
		border-radius: 4px;
		color: var(--text-subtle);
		transition:
			color var(--dur-fast) var(--ease-out),
			background-color var(--dur-fast) var(--ease-out),
			transform var(--dur-fast) var(--ease-out);
	}
	.heart:hover {
		background: var(--surface-raised);
		color: var(--text);
	}
	.heart:active {
		transform: scale(0.9);
	}
	.heart.on {
		color: #ff5a6e;
	}
	.heart:focus-visible {
		outline: 2px solid var(--focus);
		outline-offset: 1px;
	}
	.select-box {
		position: absolute;
		top: 8px;
		left: 8px;
		display: flex;
		width: 20px;
		height: 20px;
		align-items: center;
		justify-content: center;
		border-radius: 4px;
		border: 1px solid rgb(255 255 255 / 0.7);
		background: rgb(0 0 0 / 0.55);
		color: #0a0a0a;
		opacity: 0;
		transition:
			opacity var(--dur-fast) var(--ease-out),
			background-color var(--dur-fast) var(--ease-out);
	}
	.clip-card:hover .select-box,
	.clip-card.selecting .select-box,
	.select-box:focus-visible {
		opacity: 1;
	}
	.clip-card.selected .select-box {
		background: var(--text);
		border-color: var(--text);
		opacity: 1;
	}
</style>
