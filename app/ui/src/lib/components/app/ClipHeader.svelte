<script lang="ts">
	import { tick, type Snippet } from 'svelte';
	import ChevronLeftIcon from '@lucide/svelte/icons/chevron-left';
	import FolderOpenIcon from '@lucide/svelte/icons/folder-open';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import HeartIcon from '@lucide/svelte/icons/heart';
	import InfoIcon from '@lucide/svelte/icons/info';
	import type { Clip } from '$lib/api/types';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import * as AlertDialog from '$lib/components/ui/alert-dialog';
	import IconButton from '$lib/components/app/IconButton.svelte';
	import AddToAlbumMenu from '$lib/components/app/albums/AddToAlbumMenu.svelte';
	import { app, backToLibrary, deleteClip, isFavorite, renameClip, revealClip, showProperties, skipsBin, toggleFavorite } from '$lib/stores/app.svelte';
	import { CODEC_LABELS, clipTitle, formatBytes, formatFormat, formatResolution, gameLabel } from '$lib/format/format';

	/** Header for the editor and the image viewer: back, name (renamable), details and file actions. */
	let { clip, busy = false, tools }: { clip: Clip; busy?: boolean; tools?: Snippet } = $props();

	let renaming = $state(false);
	let nameDraft = $state('');
	let nameInput = $state<HTMLInputElement | null>(null);
	let confirmDelete = $state(false);
	let deleting = $state(false);

	const image = $derived(clip.kind === 'image');
	const favorite = $derived(isFavorite(clip));
	const backLabel = $derived(app.returnTo === 'album' ? 'Album' : 'Clips');
	const meta = $derived(
		image
			? `${gameLabel(clip.game)} · ${formatResolution(clip.width, clip.height)} · ${clip.fileName.split('.').pop()?.toUpperCase()} · ${formatBytes(clip.sizeBytes)}`
			: `${gameLabel(clip.game)} · ${formatFormat(clip)} · ${CODEC_LABELS[clip.codec]} · ${formatBytes(clip.sizeBytes)}`
	);

	async function startRename() {
		nameDraft = clipTitle(clip);
		renaming = true;
		await tick();
		nameInput?.focus();
		nameInput?.select();
	}

	async function commitRename() {
		const name = nameDraft.trim();
		renaming = false;
		if (!name || name === clipTitle(clip)) return;
		await renameClip(clip.path, name);
	}

	async function confirmedDelete() {
		deleting = true;
		await deleteClip(clip.path);
		deleting = false;
	}
</script>

<header class="flex min-h-10 shrink-0 items-center gap-3 border-b border-line px-3">
	<Button variant="ghost" size="sm" onclick={backToLibrary} aria-keyshortcuts="Escape" title="Back (Esc)">
		<ChevronLeftIcon data-icon="inline-start" />
		{backLabel}
	</Button>
	<span class="h-4 w-px shrink-0 bg-line" aria-hidden="true"></span>

	{#if renaming}
		<Input
			bind:ref={nameInput}
			bind:value={nameDraft}
			class="w-72 max-w-full"
			aria-label="Name"
			spellcheck={false}
			autocomplete="off"
			onkeydown={(e) => {
				if (e.key === 'Enter') {
					e.preventDefault();
					void commitRename();
				} else if (e.key === 'Escape') {
					e.preventDefault();
					e.stopPropagation();
					renaming = false;
				}
			}}
			onblur={() => (renaming = false)}
		/>
	{:else}
		<h1 class="selectable min-w-0 truncate text-sm font-semibold" title={clip.path}>{clipTitle(clip)}</h1>
		<IconButton label="Rename" icon={PencilIcon} size="icon-xs" onclick={startRename} disabled={busy} />
		<button
			type="button"
			class="heart flex size-7 shrink-0 items-center justify-center rounded-sm"
			class:on={favorite}
			aria-pressed={favorite}
			aria-label={favorite ? 'Remove from Favorites' : 'Add to Favorites'}
			title={favorite ? 'Remove from Favorites' : 'Add to Favorites'}
			onclick={() => toggleFavorite(clip.path)}
		>
			<HeartIcon class="size-4" strokeWidth={favorite ? 0 : 2} fill={favorite ? 'currentColor' : 'none'} aria-hidden="true" />
		</button>
	{/if}

	<p class="tnum hidden min-w-0 truncate text-xs text-muted-foreground min-[1440px]:block">{meta}</p>

	<div class="ml-auto flex shrink-0 items-center gap-2">
		{@render tools?.()}
		<AddToAlbumMenu paths={[clip.path]} compact />
		<Button variant="outline" size="sm" onclick={() => showProperties(clip.path)} aria-label="Properties" title="Properties">
			<InfoIcon data-icon="inline-start" />
			<span class="max-[1279px]:hidden">Properties</span>
		</Button>
		<Button variant="outline" size="sm" onclick={() => revealClip(clip.path)} aria-label="Show in folder" title="Show in folder">
			<FolderOpenIcon data-icon="inline-start" />
			<span class="max-[1279px]:hidden">Show in folder</span>
		</Button>
		<Button variant="destructive" size="sm" onclick={() => (confirmDelete = true)} disabled={busy} aria-label="Delete" title="Delete">
			<Trash2Icon data-icon="inline-start" />
			<span class="max-[1279px]:hidden">Delete</span>
		</Button>
	</div>
</header>

<AlertDialog.Root bind:open={confirmDelete}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>Delete this {image ? 'screenshot' : 'clip'}?</AlertDialog.Title>
			<AlertDialog.Description class="selectable">{clip.fileName} will be {skipsBin() ? 'deleted for good' : 'moved to the Recycle Bin'}.</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>Cancel</AlertDialog.Cancel>
			<AlertDialog.Action variant="destructive" disabled={deleting} onclick={confirmedDelete}>Delete</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>

<style>
	.heart {
		color: var(--text-subtle);
		transition:
			color var(--dur-fast) var(--ease-out),
			background-color var(--dur-fast) var(--ease-out);
	}
	.heart:hover {
		background: var(--surface-raised);
		color: var(--text);
	}
	.heart.on {
		color: #ff5a6e;
	}
</style>
