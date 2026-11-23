<script lang="ts">
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import XIcon from '@lucide/svelte/icons/x';
	import FolderMinusIcon from '@lucide/svelte/icons/folder-minus';
	import ShrinkIcon from '@lucide/svelte/icons/shrink';
	import LayersIcon from '@lucide/svelte/icons/layers';
	import { burnChoices } from '$lib/format/burn';
	import * as DropdownMenu from '$lib/components/ui/dropdown-menu';
	import { COMPRESS_QUALITIES } from '$lib/format/compress';
	import { Button } from '$lib/components/ui/button';
	import * as AlertDialog from '$lib/components/ui/alert-dialog';
	import AddToAlbumMenu from '$lib/components/app/albums/AddToAlbumMenu.svelte';
	import { app, burnIn, compressClips, deleteClips, layersOf, removeFromAlbum, skipsBin } from '$lib/stores/app.svelte';
	import { clearSelection, selection } from '$lib/stores/selection.svelte';
	import { plural } from '$lib/format/format';

	/** Actions for the selected clips. Shown in place of the toolbar, so nothing below moves. */
	let { albumId = null }: { albumId?: string | null } = $props();

	let confirming = $state(false);
	let busy = $state(false);

	const count = $derived(selection.paths.length);
	/** Screenshots cannot be compressed; only the videos among the selection are. */
	const videos = $derived(selection.paths.filter((p) => app.clips.find((c) => c.path === p)?.kind !== 'image'));
	const usual = $derived(app.settings?.storage.shrink.quality ?? 'balanced');
	const burns = $derived(burnChoices(layersOf(videos)));

	async function confirmDelete() {
		busy = true;
		await deleteClips([...selection.paths]);
		busy = false;
	}
</script>

<div class="flex min-h-12 shrink-0 flex-wrap items-center gap-2 border-b border-line bg-panel px-4 py-2" role="toolbar" aria-label="Selection">
	<p class="tnum mr-2 text-sm font-medium text-foreground" aria-live="polite">{count} selected</p>
	<AddToAlbumMenu paths={selection.paths} />
	{#if albumId}
		<Button variant="outline" size="sm" onclick={() => albumId && removeFromAlbum(albumId, [...selection.paths])}>
			<FolderMinusIcon data-icon="inline-start" />
			Remove from album
		</Button>
	{/if}
	<DropdownMenu.Root>
		<DropdownMenu.Trigger>
			{#snippet child({ props })}
				<Button {...props} variant="outline" size="sm" disabled={videos.length === 0} title="Encode them again, smaller">
					<ShrinkIcon data-icon="inline-start" />
					Compress
				</Button>
			{/snippet}
		</DropdownMenu.Trigger>
		<DropdownMenu.Content align="start" class="min-w-56">
			<DropdownMenu.Group>
				<DropdownMenu.Label>Compress {videos.length === 1 ? 'the video' : `${videos.length} videos`}</DropdownMenu.Label>
				{#each COMPRESS_QUALITIES as q (q.value)}
					<DropdownMenu.Item onSelect={() => compressClips(videos, q.value)}>
						<span class="flex flex-col">
							<span>{q.label}{q.value === usual ? ' (your setting)' : ''}</span>
							<span class="text-xs text-muted-foreground">{q.detail}</span>
						</span>
					</DropdownMenu.Item>
				{/each}
			</DropdownMenu.Group>
		</DropdownMenu.Content>
	</DropdownMenu.Root>
	{#if burns.length > 0}
		<DropdownMenu.Root>
			<DropdownMenu.Trigger>
				{#snippet child({ props })}
					<Button {...props} variant="outline" size="sm" title="Draw their camera, keyboard and mouse or logo into the picture">
						<LayersIcon data-icon="inline-start" />
						Burn in
					</Button>
				{/snippet}
			</DropdownMenu.Trigger>
			<DropdownMenu.Content align="start" class="min-w-48">
				<DropdownMenu.Group>
					<DropdownMenu.Label>Burn into the picture</DropdownMenu.Label>
					{#each burns as choice (choice.label)}
						<DropdownMenu.Item onSelect={() => burnIn(videos, choice.layers)}>{choice.label}</DropdownMenu.Item>
					{/each}
				</DropdownMenu.Group>
			</DropdownMenu.Content>
		</DropdownMenu.Root>
	{/if}
	<Button variant="destructive" size="sm" onclick={() => (confirming = true)}>
		<Trash2Icon data-icon="inline-start" />
		Delete
	</Button>
	<Button variant="ghost" size="sm" class="ml-auto" onclick={clearSelection} title="Clear the selection (Esc)">
		<XIcon data-icon="inline-start" />
		Clear
	</Button>
</div>

<AlertDialog.Root bind:open={confirming}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>Delete {plural(count, 'item')}?</AlertDialog.Title>
			<AlertDialog.Description>
				{count === 1 ? 'It' : 'They'} will be {skipsBin() ? 'deleted for good' : 'moved to the Recycle Bin'} and removed from every album.
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>Cancel</AlertDialog.Cancel>
			<AlertDialog.Action variant="destructive" disabled={busy} onclick={confirmDelete}>Delete</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
