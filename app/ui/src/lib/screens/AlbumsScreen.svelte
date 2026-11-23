<script lang="ts">
	import PlusIcon from '@lucide/svelte/icons/plus';
	import EllipsisIcon from '@lucide/svelte/icons/ellipsis';
	import EyeOffIcon from '@lucide/svelte/icons/eye-off';
	import HeartIcon from '@lucide/svelte/icons/heart';
	import type { Album } from '$lib/api/types';
	import { Button } from '$lib/components/ui/button';
	import * as AlertDialog from '$lib/components/ui/alert-dialog';
	import * as DropdownMenu from '$lib/components/ui/dropdown-menu';
	import * as Empty from '$lib/components/ui/empty';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import AlbumNameDialog from '$lib/components/app/albums/AlbumNameDialog.svelte';
	import {
		albumCover,
		app,
		createAlbum,
		deleteAlbum,
		openAlbum,
		renameAlbum,
		setAlbumHidden
	} from '$lib/stores/app.svelte';
	import { plural } from '$lib/format/format';

	let creating = $state(false);
	let renaming = $state<Album | null>(null);
	let renameOpen = $state(false);
	let deleting = $state<Album | null>(null);

	// Favorites first, then by name.
	const sorted = $derived(
		[...app.albums].sort((a, b) => Number(b.builtin) - Number(a.builtin) || a.name.localeCompare(b.name))
	);

	async function create(name: string) {
		const album = await createAlbum(name);
		if (album) openAlbum(album.id);
		return album !== null;
	}
</script>

<section class="flex min-h-0 flex-1 flex-col" aria-labelledby="albums-title">
	<header class="flex min-h-12 shrink-0 items-center gap-3 border-b border-line px-4">
		<h1 id="albums-title" class="text-base font-semibold">Albums</h1>
		<span class="tnum text-xs text-muted-foreground">{plural(app.albums.length, 'album')}</span>
		<Button size="sm" class="ml-auto" onclick={() => (creating = true)}>
			<PlusIcon data-icon="inline-start" />
			New album
		</Button>
	</header>

	<div class="min-h-0 flex-1 overflow-y-auto">
		{#if app.albumsState === 'loading'}
			<div class="grid grid-cols-[repeat(auto-fill,minmax(200px,1fr))] gap-4 p-4" aria-busy="true">
				{#each { length: 4 } as _, i (i)}
					<div class="flex flex-col gap-2"><Skeleton class="aspect-video w-full rounded-md" /><Skeleton class="h-4 w-1/2" /></div>
				{/each}
			</div>
		{:else if app.albums.length === 0}
			<Empty.Root class="h-full border-0">
				<Empty.Header>
					<Empty.Title>No albums yet</Empty.Title>
					<Empty.Description>
						Albums group clips without moving files. Select clips on the Clips screen and choose Add to album, or start one here.
					</Empty.Description>
				</Empty.Header>
				<Empty.Content>
					<Button variant="outline" onclick={() => (creating = true)}>New album</Button>
				</Empty.Content>
			</Empty.Root>
		{:else}
			<ul class="grid grid-cols-[repeat(auto-fill,minmax(200px,1fr))] gap-x-4 gap-y-6 p-4" aria-label="Albums">
				{#each sorted as album (album.id)}
					{@const cover = albumCover(album)}
					<li class="album relative min-w-0">
						<button type="button" class="album-main flex w-full flex-col gap-2 rounded-md text-left outline-none" onclick={() => openAlbum(album.id)}>
							<span class="thumb relative block aspect-video w-full overflow-hidden rounded-md border border-line bg-raised">
								{#if cover}
									<img src={cover} alt="" class="size-full object-cover" loading="lazy" draggable="false" />
								{/if}
							</span>
							<span class="flex min-w-0 flex-col">
								<span class="flex min-w-0 items-center gap-1.5 text-sm font-medium text-foreground">
									{#if album.builtin}<HeartIcon class="size-3.5 shrink-0 fill-current" aria-hidden="true" />{/if}
									<span class="truncate">{album.name}</span>
								</span>
								<span class="flex items-center gap-1 text-xs text-muted-foreground">
									<span class="tnum">{plural(album.clipPaths.length, 'item')}</span>
									{#if album.hideFromClips}
										<span aria-hidden="true">&middot;</span>
										<EyeOffIcon class="size-3" aria-hidden="true" />
										<span>Hidden from Clips</span>
									{/if}
								</span>
							</span>
						</button>
						<DropdownMenu.Root>
							<DropdownMenu.Trigger>
								{#snippet child({ props })}
									<Button {...props} variant="secondary" size="icon-xs" class="menu-btn absolute top-2 right-2" aria-label="Actions for {album.name}">
										<EllipsisIcon />
									</Button>
								{/snippet}
							</DropdownMenu.Trigger>
							<DropdownMenu.Content align="end" class="w-56">
								<DropdownMenu.Group>
									{#if !album.builtin}
										<DropdownMenu.Item onSelect={() => {
											renaming = album;
											renameOpen = true;
										}}>Rename…</DropdownMenu.Item>
									{/if}
									<DropdownMenu.CheckboxItem checked={album.hideFromClips} onCheckedChange={(v) => setAlbumHidden(album.id, v)}>
										Hide these clips from Clips
									</DropdownMenu.CheckboxItem>
								</DropdownMenu.Group>
								{#if !album.builtin}
									<DropdownMenu.Separator />
									<DropdownMenu.Group>
										<DropdownMenu.Item variant="destructive" onSelect={() => (deleting = album)}>Delete album…</DropdownMenu.Item>
									</DropdownMenu.Group>
								{/if}
							</DropdownMenu.Content>
						</DropdownMenu.Root>
					</li>
				{/each}
			</ul>
		{/if}
	</div>
</section>

<AlbumNameDialog bind:open={creating} title="New album" confirmLabel="Create" onsubmit={create} />
<AlbumNameDialog
	bind:open={renameOpen}
	title="Rename album"
	confirmLabel="Rename"
	initial={renaming?.name ?? ''}
	onsubmit={async (name) => (renaming ? renameAlbum(renaming.id, name) : false)}
/>

<AlertDialog.Root open={deleting !== null} onOpenChange={(o) => !o && (deleting = null)}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>Delete “{deleting?.name}”?</AlertDialog.Title>
			<AlertDialog.Description>Only the album is deleted. Its clips stay in the library.</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>Cancel</AlertDialog.Cancel>
			<AlertDialog.Action variant="destructive" onclick={() => deleting && deleteAlbum(deleting.id).then(() => (deleting = null))}>Delete album</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>

<style>
	.thumb {
		transition: border-color var(--dur-base) var(--ease-out);
	}
	.album:hover .thumb {
		border-color: var(--text-subtle);
	}
	.album-main:focus-visible .thumb {
		outline: 2px solid var(--focus);
		outline-offset: 2px;
	}
	.album :global(.menu-btn) {
		opacity: 0;
		transition: opacity var(--dur-fast) var(--ease-out);
	}
	.album:hover :global(.menu-btn),
	.album :global(.menu-btn:focus-visible),
	.album :global(.menu-btn[aria-expanded='true']) {
		opacity: 1;
	}
</style>
