<script lang="ts">
	import FolderPlusIcon from '@lucide/svelte/icons/folder-plus';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import CheckIcon from '@lucide/svelte/icons/check';
	import { Button, type ButtonVariant } from '$lib/components/ui/button';
	import * as DropdownMenu from '$lib/components/ui/dropdown-menu';
	import { addToAlbum, app, createAlbum } from '$lib/stores/app.svelte';
	import AlbumNameDialog from './AlbumNameDialog.svelte';

	/** "Add to album": a menu of albums plus "New album…", for one or many clips. */
	let {
		paths,
		variant = 'outline',
		compact = false,
		ondone
	}: { paths: string[]; variant?: ButtonVariant; compact?: boolean; ondone?: () => void } = $props();

	let naming = $state(false);

	const containsAll = (clipPaths: string[]) => paths.length > 0 && paths.every((p) => clipPaths.includes(p));

	async function add(id: string) {
		if (await addToAlbum(id, paths)) ondone?.();
	}

	async function createAndAdd(name: string): Promise<boolean> {
		const album = await createAlbum(name);
		if (!album) return false;
		await add(album.id);
		return true;
	}
</script>

<DropdownMenu.Root>
	<DropdownMenu.Trigger>
		{#snippet child({ props })}
			<Button {...props} {variant} size="sm" disabled={paths.length === 0} aria-label="Add to album" title="Add to album">
				<FolderPlusIcon data-icon="inline-start" />
				<span class={compact ? 'max-[1279px]:hidden' : ''}>Add to album</span>
			</Button>
		{/snippet}
	</DropdownMenu.Trigger>
	<DropdownMenu.Content align="start" class="w-56">
		<DropdownMenu.Group>
			<DropdownMenu.GroupHeading>Albums</DropdownMenu.GroupHeading>
			{#each app.albums as album (album.id)}
				{@const already = containsAll(album.clipPaths)}
				<DropdownMenu.Item disabled={already} onSelect={() => add(album.id)}>
					<span class="truncate">{album.name}</span>
					{#if already}
						<CheckIcon class="ml-auto" aria-label="Already in this album" />
					{:else}
						<span class="tnum ml-auto text-xs text-subtle">{album.clipPaths.length}</span>
					{/if}
				</DropdownMenu.Item>
			{:else}
				<DropdownMenu.Item disabled>No albums yet</DropdownMenu.Item>
			{/each}
		</DropdownMenu.Group>
		<DropdownMenu.Separator />
		<DropdownMenu.Group>
			<DropdownMenu.Item onSelect={() => (naming = true)}>
				<PlusIcon />
				New album…
			</DropdownMenu.Item>
		</DropdownMenu.Group>
	</DropdownMenu.Content>
</DropdownMenu.Root>

<AlbumNameDialog bind:open={naming} title="New album" confirmLabel="Create and add" onsubmit={createAndAdd} />
