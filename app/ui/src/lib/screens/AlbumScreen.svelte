<script lang="ts">
	import ChevronLeftIcon from '@lucide/svelte/icons/chevron-left';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import LayoutGridIcon from '@lucide/svelte/icons/layout-grid';
	import ListIcon from '@lucide/svelte/icons/list';
	import { Button } from '$lib/components/ui/button';
	import * as AlertDialog from '$lib/components/ui/alert-dialog';
	import * as Empty from '$lib/components/ui/empty';
	import * as Select from '$lib/components/ui/select';
	import { Switch } from '$lib/components/ui/switch';
	import { Label } from '$lib/components/ui/label';
	import * as ToggleGroup from '$lib/components/ui/toggle-group';
	import IconButton from '$lib/components/app/IconButton.svelte';
	import AlbumNameDialog from '$lib/components/app/albums/AlbumNameDialog.svelte';
	import ClipLibrary from '$lib/components/app/clips/ClipLibrary.svelte';
	import SelectionBar from '$lib/components/app/clips/SelectionBar.svelte';
	import { app, currentAlbum, deleteAlbum, go, renameAlbum, setAlbumHidden } from '$lib/stores/app.svelte';
	import { selection } from '$lib/stores/selection.svelte';
	import { SORT_OPTIONS, persistView, view, type ViewMode } from '$lib/stores/view.svelte';
	import { sortClips } from '$lib/stores/sort';
	import { formatBytes, plural } from '$lib/format/format';

	const album = $derived(currentAlbum());
	const clips = $derived.by(() => {
		if (!album) return [];
		const members = new Set(album.clipPaths);
		return sortClips(app.clips.filter((c) => members.has(c.path)), view.sort);
	});
	const totalBytes = $derived(clips.reduce((sum, c) => sum + c.sizeBytes, 0));
	const sortLabel = $derived(SORT_OPTIONS.find((o) => o.value === view.sort)?.label ?? 'Sort');

	let renaming = $state(false);
	let confirmDelete = $state(false);
</script>

{#if album}
	<section class="flex min-h-0 flex-1 flex-col" aria-labelledby="album-title">
		<header class="flex min-h-12 shrink-0 flex-wrap items-center gap-3 border-b border-line px-4 py-2">
			<Button variant="ghost" size="sm" onclick={() => go('albums')}>
				<ChevronLeftIcon data-icon="inline-start" />
				Albums
			</Button>
			<span class="h-4 w-px bg-line" aria-hidden="true"></span>
			<h1 id="album-title" class="selectable min-w-0 truncate text-base font-semibold">{album.name}</h1>
			{#if !album.builtin}
				<IconButton label="Rename album" icon={PencilIcon} size="icon-xs" onclick={() => (renaming = true)} />
			{/if}
			<div class="ml-auto flex items-center gap-3">
				<div class="flex items-center gap-2">
					<Switch id="album-hidden" size="sm" checked={album.hideFromClips} onCheckedChange={(v) => setAlbumHidden(album.id, v)} />
					<Label for="album-hidden" class="text-xs font-normal text-muted-foreground">Hide these clips from Clips</Label>
				</div>
				{#if !album.builtin}
					<Button variant="destructive" size="sm" onclick={() => (confirmDelete = true)} aria-label="Delete album" title="Delete album">
						<Trash2Icon data-icon="inline-start" />
						<span class="max-[1279px]:hidden">Delete album</span>
					</Button>
				{/if}
			</div>
		</header>

		{#if selection.paths.length > 0}
			<SelectionBar albumId={album.id} />
		{:else}
			<div class="flex min-h-12 shrink-0 items-center gap-2 border-b border-line px-4 py-2" role="toolbar" aria-label="Album view">
				<p class="tnum text-xs text-muted-foreground">{plural(clips.length, 'item')} &middot; {formatBytes(totalBytes)}</p>
				<div class="ml-auto flex items-center gap-2">
					<Select.Root type="single" bind:value={view.sort} onValueChange={persistView}>
						<Select.Trigger size="sm" class="w-36" aria-label="Sort">
							<span class="truncate">{sortLabel}</span>
						</Select.Trigger>
						<Select.Content align="end">
							{#each SORT_OPTIONS as option (option.value)}
								<Select.Item value={option.value} label={option.label}>{option.label}</Select.Item>
							{/each}
						</Select.Content>
					</Select.Root>
					<ToggleGroup.Root
						type="single"
						variant="outline"
						size="sm"
						value={view.mode}
						onValueChange={(v) => {
							if (v) {
								view.mode = v as ViewMode;
								persistView();
							}
						}}
						aria-label="Layout"
					>
						<ToggleGroup.Item value="grid" aria-label="Grid" title="Grid"><LayoutGridIcon /></ToggleGroup.Item>
						<ToggleGroup.Item value="list" aria-label="List" title="List"><ListIcon /></ToggleGroup.Item>
					</ToggleGroup.Root>
				</div>
			</div>
		{/if}

		<div class="min-h-0 flex-1 overflow-y-auto">
			{#if clips.length === 0}
				<Empty.Root class="h-full border-0">
					<Empty.Header>
						<Empty.Title>This album is empty</Empty.Title>
						<Empty.Description>On the Clips screen, select clips and choose Add to album.</Empty.Description>
					</Empty.Header>
					<Empty.Content>
						<Button variant="outline" onclick={() => go('clips')}>Go to clips</Button>
					</Empty.Content>
				</Empty.Root>
			{:else}
				<ClipLibrary {clips} mode={view.mode} grouped={view.sort === 'newest' || view.sort === 'oldest'} />
			{/if}
		</div>
	</section>

	<AlbumNameDialog bind:open={renaming} title="Rename album" confirmLabel="Rename" initial={album.name} onsubmit={(name) => renameAlbum(album.id, name)} />

	<AlertDialog.Root bind:open={confirmDelete}>
		<AlertDialog.Content>
			<AlertDialog.Header>
				<AlertDialog.Title>Delete “{album.name}”?</AlertDialog.Title>
				<AlertDialog.Description>Only the album is deleted. Its {plural(clips.length, 'item')} stay in the library.</AlertDialog.Description>
			</AlertDialog.Header>
			<AlertDialog.Footer>
				<AlertDialog.Cancel>Cancel</AlertDialog.Cancel>
				<AlertDialog.Action variant="destructive" onclick={() => deleteAlbum(album.id)}>Delete album</AlertDialog.Action>
			</AlertDialog.Footer>
		</AlertDialog.Content>
	</AlertDialog.Root>
{:else}
	<Empty.Root class="flex-1 border-0">
		<Empty.Header>
			<Empty.Title>Album not found</Empty.Title>
			<Empty.Description>It may have been deleted.</Empty.Description>
		</Empty.Header>
		<Empty.Content>
			<Button variant="outline" onclick={() => go('albums')}>Back to albums</Button>
		</Empty.Content>
	</Empty.Root>
{/if}
