<script lang="ts">
	import type { Snippet } from 'svelte';
	import { ContextMenu } from 'bits-ui';
	import PlayIcon from '@lucide/svelte/icons/play';
	import ImageIcon from '@lucide/svelte/icons/image';
	import ScissorsIcon from '@lucide/svelte/icons/scissors';
	import HeartIcon from '@lucide/svelte/icons/heart';
	import CopyIcon from '@lucide/svelte/icons/copy';
	import FolderOpenIcon from '@lucide/svelte/icons/folder-open';
	import InfoIcon from '@lucide/svelte/icons/info';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import ShrinkIcon from '@lucide/svelte/icons/shrink';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import LayersIcon from '@lucide/svelte/icons/layers';
	import { COMPRESS_QUALITIES } from '$lib/format/compress';
	import { burnChoices } from '$lib/format/burn';
	import { app, burnIn, compressClips as compress, layersOf } from '$lib/stores/app.svelte';
	import { selection } from '$lib/stores/selection.svelte';
	import type { Clip } from '$lib/api/types';
	import * as AlertDialog from '$lib/components/ui/alert-dialog';
	import {
		copyClipFiles,
		deleteClip,
		isFavorite,
		openClip,
		openInEditor,
		revealClip,
		showProperties,
		skipsBin,
		toggleFavorite
	} from '$lib/stores/app.svelte';
	import { clipTitle } from '$lib/format/format';

	/**
	 * Right-click menu for a clip in the library. `trigger` renders the element that opens it
	 * (a card or a table row) with the given props spread on it.
	 */
	let { clip, list, trigger }: { clip: Clip; list: string[]; trigger: Snippet<[Record<string, unknown>]> } = $props();

	const image = $derived(clip.kind === 'image');
	/** Right-clicking one of several selected clips acts on all of them. */
	const targets = $derived(selection.paths.length > 1 && selection.paths.includes(clip.path) ? [...selection.paths] : [clip.path]);
	const usual = $derived(app.settings?.storage.shrink.quality ?? 'balanced');
	/** Burn in only offers the layers the clips have; worked out when the menu opens. */
	let open = $state(false);
	const burns = $derived(open && !image ? burnChoices(layersOf(targets)) : []);
	let confirmDelete = $state(false);

	const item =
		'flex cursor-default items-center gap-2 rounded-md px-2 py-1.5 text-sm outline-hidden select-none data-highlighted:bg-accent data-highlighted:text-accent-foreground [&_svg]:size-4 [&_svg]:shrink-0';
</script>

<ContextMenu.Root bind:open>
	<ContextMenu.Trigger>
		{#snippet child({ props })}
			{@render trigger(props)}
		{/snippet}
	</ContextMenu.Trigger>
	<ContextMenu.Portal>
		<ContextMenu.Content
			class="z-50 min-w-52 origin-(--bits-context-menu-content-transform-origin) rounded-lg bg-popover p-1 text-popover-foreground shadow-md ring-1 ring-foreground/10 data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 duration-100"
		>
			<ContextMenu.Item class={item} onSelect={() => openClip(clip.path, list)}>
				{#if image}<ImageIcon aria-hidden="true" /> View{:else}<PlayIcon aria-hidden="true" /> Play{/if}
			</ContextMenu.Item>
			{#if !image}
				<ContextMenu.Item class={item} onSelect={() => openInEditor(clip.path, 0)}>
					<ScissorsIcon aria-hidden="true" /> Edit
				</ContextMenu.Item>
			{/if}
			<ContextMenu.Separator class="-mx-1 my-1 h-px bg-border" />
			<ContextMenu.Item class={item} onSelect={() => toggleFavorite(clip.path)}>
				<HeartIcon aria-hidden="true" class={isFavorite(clip) ? 'fill-current text-[#ff5a6e]' : ''} />
				{isFavorite(clip) ? 'Remove from Favorites' : 'Add to Favorites'}
			</ContextMenu.Item>
			<ContextMenu.Item class={item} onSelect={() => copyClipFiles([clip.path])}>
				<CopyIcon aria-hidden="true" /> Copy file
			</ContextMenu.Item>
			<ContextMenu.Item class={item} onSelect={() => revealClip(clip.path)}>
				<FolderOpenIcon aria-hidden="true" /> Show in folder
			</ContextMenu.Item>
			{#if !image}
				<ContextMenu.Sub>
					<ContextMenu.SubTrigger class={item}>
						<ShrinkIcon aria-hidden="true" />
						{targets.length > 1 ? `Compress ${targets.length} clips` : 'Compress'}
						<ChevronRightIcon class="ml-auto" aria-hidden="true" />
					</ContextMenu.SubTrigger>
					<ContextMenu.SubContent
						class="z-50 min-w-56 rounded-lg bg-popover p-1 text-popover-foreground shadow-md ring-1 ring-foreground/10"
						sideOffset={4}
					>
						{#each COMPRESS_QUALITIES as q (q.value)}
							<ContextMenu.Item class={item} onSelect={() => compress(targets, q.value)}>
								<span class="flex flex-col">
									<span>{q.label}{q.value === usual ? ' (your setting)' : ''}</span>
									<span class="text-xs text-muted-foreground">{q.detail}</span>
								</span>
							</ContextMenu.Item>
						{/each}
					</ContextMenu.SubContent>
				</ContextMenu.Sub>
			{/if}
			{#if burns.length > 0}
				<ContextMenu.Sub>
					<ContextMenu.SubTrigger class={item}>
						<LayersIcon aria-hidden="true" />
						{targets.length > 1 ? `Burn in ${targets.length} clips` : 'Burn in'}
						<ChevronRightIcon class="ml-auto" aria-hidden="true" />
					</ContextMenu.SubTrigger>
					<ContextMenu.SubContent
						class="z-50 min-w-48 rounded-lg bg-popover p-1 text-popover-foreground shadow-md ring-1 ring-foreground/10"
						sideOffset={4}
					>
						{#each burns as choice (choice.label)}
							<ContextMenu.Item class={item} onSelect={() => burnIn(targets, choice.layers)}>{choice.label}</ContextMenu.Item>
						{/each}
					</ContextMenu.SubContent>
				</ContextMenu.Sub>
			{/if}
			<ContextMenu.Item class={item} onSelect={() => showProperties(clip.path)}>
				<InfoIcon aria-hidden="true" /> Properties
			</ContextMenu.Item>
			<ContextMenu.Separator class="-mx-1 my-1 h-px bg-border" />
			<ContextMenu.Item class="{item} text-destructive data-highlighted:bg-destructive/15 data-highlighted:text-destructive" onSelect={() => (confirmDelete = true)}>
				<Trash2Icon aria-hidden="true" /> Delete…
			</ContextMenu.Item>
		</ContextMenu.Content>
	</ContextMenu.Portal>
</ContextMenu.Root>

<AlertDialog.Root bind:open={confirmDelete}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>Delete {clipTitle(clip)}?</AlertDialog.Title>
			<AlertDialog.Description>
				{skipsBin() ? 'It is deleted for good and cannot be brought back.' : 'It goes to the Recycle Bin, so you can still get it back from there.'}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>Cancel</AlertDialog.Cancel>
			<AlertDialog.Action variant="destructive" onclick={() => deleteClip(clip.path)}>Delete</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
