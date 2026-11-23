<script lang="ts">
	import { dragOut } from '$lib/actions/dragOut';
	import ImageIcon from '@lucide/svelte/icons/image';
	import HeartIcon from '@lucide/svelte/icons/heart';
	import type { Clip } from '$lib/api/types';
	import { Checkbox } from '$lib/components/ui/checkbox';
	import {
		CODEC_LABELS,
		clipTitle,
		formatBytes,
		formatDuration,
		formatFormat,
		formatFullDate,
		formatResolution,
		formatWhen,
		gameLabel
	} from '$lib/format/format';
	import { isSelected } from '$lib/stores/selection.svelte';
	import { isFavorite, toggleFavorite } from '$lib/stores/app.svelte';
	import ClipContextMenu from './ClipContextMenu.svelte';
	import LayerBadges from './LayerBadges.svelte';
	import { dayKey, dayLabel } from '$lib/format/days';

	let {
		clips,
		onactivate,
		ontoggle,
		grouped = false
	}: {
		clips: Clip[];
		/** Rows under a heading per day (when sorted by date). */
		grouped?: boolean;
		onactivate: (clip: Clip, event: MouseEvent) => void;
		ontoggle: (clip: Clip) => void;
	} = $props();

	const paths = $derived(clips.map((c) => c.path));
</script>

<table class="w-full table-fixed border-separate border-spacing-0 text-sm">
	<caption class="sr-only">Clips and screenshots</caption>
	<thead>
		<tr class="text-left text-xs text-subtle">
			<th scope="col" class="sticky top-0 z-10 w-10 border-b border-line bg-canvas px-2 py-2"><span class="sr-only">Selected</span></th>
			<th scope="col" class="sticky top-0 z-10 w-9 border-b border-line bg-canvas py-2"><span class="sr-only">Favorite</span></th>
			<th scope="col" class="sticky top-0 z-10 border-b border-line bg-canvas px-2 py-2 font-medium">Name</th>
			<th scope="col" class="sticky top-0 z-10 w-36 border-b border-line bg-canvas px-2 py-2 font-medium">Game</th>
			<th scope="col" class="sticky top-0 z-10 w-32 border-b border-line bg-canvas px-2 py-2 font-medium">Date</th>
			<th scope="col" class="sticky top-0 z-10 w-20 border-b border-line bg-canvas px-2 py-2 text-right font-medium">Length</th>
			<th scope="col" class="sticky top-0 z-10 hidden w-44 border-b border-line bg-canvas px-2 py-2 font-medium min-[1280px]:table-cell">Format</th>
			<th scope="col" class="sticky top-0 z-10 hidden w-16 border-b border-line bg-canvas px-2 py-2 font-medium min-[1280px]:table-cell">Codec</th>
			<th scope="col" class="sticky top-0 z-10 w-20 border-b border-line bg-canvas px-2 py-2 text-right font-medium">Size</th>
		</tr>
	</thead>
	<tbody>
		{#each clips as clip, i (clip.path)}
			{@const selected = isSelected(clip.path)}
			{#if grouped && (i === 0 || dayKey(clips[i - 1].createdAt) !== dayKey(clip.createdAt))}
				<tr class="day-row" aria-hidden="false">
					<th scope="rowgroup" colspan="9" class="px-2 pt-4 pb-1.5 text-left text-xs font-medium text-muted-foreground">
						{dayLabel(clip.createdAt)}
						<span class="tnum ml-1 font-normal text-subtle">{clips.filter((c) => dayKey(c.createdAt) === dayKey(clip.createdAt)).length}</span>
					</th>
				</tr>
			{/if}
			{@const image = clip.kind === 'image'}
			{@const favorite = isFavorite(clip)}
			<ClipContextMenu {clip} list={paths}>
				{#snippet trigger(props)}
					<tr class="clip-row cursor-pointer" class:selected data-clip-path={clip.path} use:dragOut={clip.path} onclick={(e) => onactivate(clip, e)} {...props}>
						<td class="border-b border-line px-2 py-1">
							<!-- svelte-ignore a11y_click_events_have_key_events -->
							<!-- svelte-ignore a11y_no_static_element_interactions -->
							<span class="flex items-center justify-center" onclick={(e) => e.stopPropagation()}>
								<Checkbox checked={selected} onCheckedChange={() => ontoggle(clip)} aria-label="Select {clipTitle(clip)}" />
							</span>
						</td>
						<td class="border-b border-line py-1">
							<button
								type="button"
								class="heart flex size-7 items-center justify-center rounded-sm"
								class:on={favorite}
								aria-pressed={favorite}
								aria-label={favorite ? `Remove ${clipTitle(clip)} from Favorites` : `Add ${clipTitle(clip)} to Favorites`}
								title={favorite ? 'Remove from Favorites' : 'Add to Favorites'}
								onclick={(e) => {
									e.stopPropagation();
									toggleFavorite(clip.path);
								}}
							>
								<HeartIcon class="size-4" strokeWidth={favorite ? 0 : 2} fill={favorite ? 'currentColor' : 'none'} aria-hidden="true" />
							</button>
						</td>
						<td class="border-b border-line px-2 py-1">
							<button
								type="button"
								class="clip-row-button flex w-full min-w-0 items-center gap-3 rounded-sm text-left"
								data-clip-card
								onclick={(e) => {
									e.stopPropagation();
									onactivate(clip, e);
								}}
								title={clip.fileName}
							>
								<span class="relative block h-9 w-16 shrink-0 overflow-hidden rounded-xs border border-line bg-raised">
									{#if clip.thumbnailUrl}
										<img src={clip.thumbnailUrl} alt="" class="size-full object-cover" loading="lazy" decoding="async" draggable="false" />
									{/if}
								</span>
								<span class="truncate font-medium text-foreground">{clipTitle(clip)}</span>
								<LayerBadges {clip} class="shrink-0 text-subtle" />
							</button>
						</td>
						<td class="border-b border-line px-2 py-1 text-muted-foreground"><span class="block truncate">{gameLabel(clip.game)}</span></td>
						<td class="border-b border-line px-2 py-1 text-muted-foreground">
							<time class="block truncate" datetime={clip.createdAt} title={formatFullDate(clip.createdAt)}>{formatWhen(clip.createdAt)}</time>
						</td>
						<td class="tnum border-b border-line px-2 py-1 text-right text-foreground">
							{#if image}
								<span class="inline-flex items-center gap-1 text-muted-foreground"><ImageIcon class="size-3" aria-hidden="true" />Still</span>
							{:else}
								{formatDuration(clip.durationSecs)}
							{/if}
						</td>
						<td class="tnum hidden border-b border-line px-2 py-1 text-muted-foreground min-[1280px]:table-cell">
							<span class="block truncate">{image ? formatResolution(clip.width, clip.height) : formatFormat(clip)}</span>
						</td>
						<td class="hidden border-b border-line px-2 py-1 text-muted-foreground min-[1280px]:table-cell">
							{image ? clip.fileName.split('.').pop()?.toUpperCase() : CODEC_LABELS[clip.codec]}
						</td>
						<td class="tnum border-b border-line px-2 py-1 text-right text-muted-foreground">{formatBytes(clip.sizeBytes)}</td>
					</tr>
				{/snippet}
			</ClipContextMenu>
		{/each}
	</tbody>
</table>

<style>
	.heart {
		color: var(--text-subtle);
		transition: color var(--dur-fast) var(--ease-out), background-color var(--dur-fast) var(--ease-out);
	}
	.heart:hover {
		background: var(--surface-raised);
		color: var(--text);
	}
	.heart.on {
		color: #ff5a6e;
	}
	.clip-row {
		transition: background-color var(--dur-fast) var(--ease-out);
	}
	.clip-row:hover {
		background: var(--surface-raised);
	}
	.clip-row.selected {
		background: var(--surface-selected);
	}
	.clip-row-button:focus-visible {
		outline-offset: 0;
	}
</style>
