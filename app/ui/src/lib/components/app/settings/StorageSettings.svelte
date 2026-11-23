<script lang="ts">
	import { tick } from 'svelte';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import { toast } from 'svelte-sonner';
	import { api, errorMessage } from '$lib/api';
	import { TEMPLATE_PLACEHOLDERS, type Settings } from '$lib/api/types';
	import { app, draftIsDirty } from '$lib/stores/app.svelte';
	import { onMount } from 'svelte';
	import type { StorageUsage } from '$lib/api/types';
	import { Spinner } from '$lib/components/ui/spinner';
	import { formatBytes, plural } from '$lib/format/format';
	import NumberField from '../fields/NumberField.svelte';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { Switch } from '$lib/components/ui/switch';
	import { hasInvalidChars, nameHasSeparator, previewPath, unknownTokens } from '$lib/format/templates';
	import PathText from '../PathText.svelte';
	import SettingsRow from './SettingsRow.svelte';
	import { showAdvanced } from '$lib/stores/settingsNav.svelte';
	import SettingsSection from './SettingsSection.svelte';
	import ShrinkSettings from './ShrinkSettings.svelte';

	let { draft }: { draft: Settings } = $props();

	type Field = 'folderTemplate' | 'nameTemplate';

	let folderInput = $state<HTMLInputElement | null>(null);
	let nameInput = $state<HTMLInputElement | null>(null);
	let picking = $state(false);
	const now = new Date();

	const preview = $derived(previewPath(draft.storage, { game: 'Valorant', when: now }));

	const folderUnknown = $derived(unknownTokens(draft.storage.folderTemplate));
	const nameUnknown = $derived(unknownTokens(draft.storage.nameTemplate));
	const nameProblems = $derived.by(() => {
		const list: string[] = [];
		if (nameUnknown.length) list.push(`Unknown placeholder ${nameUnknown.join(', ')}. It is kept as typed.`);
		if (draft.storage.nameTemplate.trim() === '') list.push('The name cannot be empty. "clip" is used instead.');
		if (nameHasSeparator(draft.storage.nameTemplate)) list.push('Slashes are not allowed in a file name. They are replaced with _.');
		else if (hasInvalidChars(draft.storage.nameTemplate)) list.push('Windows does not allow < > : " | ? * in a name. They are replaced with _.');
		return list;
	});
	const folderProblems = $derived.by(() => {
		const list: string[] = [];
		if (folderUnknown.length) list.push(`Unknown placeholder ${folderUnknown.join(', ')}. It is kept as typed.`);
		if (hasInvalidChars(draft.storage.folderTemplate)) list.push('Windows does not allow < > : " | ? * in a folder name. They are replaced with _.');
		return list;
	});

	async function openSettingsFolder() {
		try {
			await api.openSettingsFolder();
		} catch (error) {
			toast.error('Could not open the settings folder', { description: errorMessage(error) });
		}
	}

	async function choose() {
		picking = true;
		try {
			const picked = await api.pickFolder();
			if (picked) draft.storage.folder = picked;
		} finally {
			picking = false;
		}
	}

	async function insert(field: Field, token: string) {
		const el = field === 'folderTemplate' ? folderInput : nameInput;
		const value = draft.storage[field];
		const start = el?.selectionStart ?? value.length;
		const end = el?.selectionEnd ?? start;
		draft.storage[field] = value.slice(0, start) + token + value.slice(end);
		await tick();
		el?.focus();
		el?.setSelectionRange(start + token.length, start + token.length);
	}

	// ---------- Storage limits ----------

	let usage = $state<StorageUsage | null>(null);
	let cleaning = $state(false);
	const usageGb = $derived(usage ? usage.bytes / 2 ** 30 : 0);

	onMount(() => {
		api.getStorageUsage().then((u) => (usage = u)).catch(() => {});
	});

	async function cleanNow() {
		cleaning = true;
		try {
			const done = await api.cleanUpStorage();
			if (done.files === 0) toast('Nothing to clean up', { description: 'Every clip is within the limits or kept.' });
			usage = await api.getStorageUsage();
		} catch (error) {
			toast.error('Could not clean up', { description: errorMessage(error) });
		} finally {
			cleaning = false;
		}
	}
</script>

<SettingsSection id="storage" title="Storage" description="Clips are saved on this computer only.">
	<SettingsRow label="Clips folder" labelId="storage-folder">
		<div class="flex items-center gap-2">
			<p class="selectable min-h-7 min-w-0 flex-1 rounded-md border border-line-control px-2 py-1 text-sm" aria-labelledby="storage-folder">{#if draft.storage.folder}<PathText path={draft.storage.folder} />{:else}No folder chosen{/if}</p>
			<Button variant="outline" onclick={choose} disabled={picking}>Choose</Button>
		</div>
	</SettingsRow>

	<SettingsRow advanced
		label="Subfolder"
		controlId="storage-subfolder"
		description="Optional. Clips go in this folder inside the clips folder. Leave empty for none."
	>
		<div class="flex flex-col gap-2">
			<Input id="storage-subfolder" bind:ref={folderInput} bind:value={draft.storage.folderTemplate} placeholder={'{game}'} autocomplete="off" spellcheck={false} />
			<div class="flex flex-wrap gap-1" role="group" aria-label="Insert a placeholder into the subfolder">
				{#each TEMPLATE_PLACEHOLDERS as p (p.token)}
					<Button variant="outline" size="xs" class="tnum" title={p.meaning} aria-label="Insert {p.token}: {p.meaning}" onmousedown={(e) => e.preventDefault()} onclick={() => insert('folderTemplate', p.token)}>{p.token}</Button>
				{/each}
			</div>
			{#each folderProblems as problem (problem)}
				<p class="flex items-start gap-2 text-xs text-foreground" role="note"><TriangleAlertIcon class="mt-px size-4 shrink-0" aria-hidden="true" />{problem}</p>
			{/each}
		</div>
	</SettingsRow>

	<SettingsRow advanced label="File name" controlId="storage-name" description="Without the extension. Clips are saved as .mp4.">
		<div class="flex flex-col gap-2">
			<Input id="storage-name" bind:ref={nameInput} bind:value={draft.storage.nameTemplate} placeholder={'{game} {date} {time}'} autocomplete="off" spellcheck={false} />
			<div class="flex flex-wrap gap-1" role="group" aria-label="Insert a placeholder into the file name">
				{#each TEMPLATE_PLACEHOLDERS as p (p.token)}
					<Button variant="outline" size="xs" class="tnum" title={p.meaning} aria-label="Insert {p.token}: {p.meaning}" onmousedown={(e) => e.preventDefault()} onclick={() => insert('nameTemplate', p.token)}>{p.token}</Button>
				{/each}
			</div>
			{#each nameProblems as problem (problem)}
				<p class="flex items-start gap-2 text-xs text-foreground" role="note"><TriangleAlertIcon class="mt-px size-4 shrink-0" aria-hidden="true" />{problem}</p>
			{/each}
		</div>
	</SettingsRow>

	<SettingsRow advanced
		label="Settings folder"
		labelId="storage-settings-folder"
		description="Where Umbra keeps its settings, albums and recording bookmarks: the Umbra folder in Documents."
	>
		<div class="flex items-center gap-2">
			<p class="selectable min-h-7 min-w-0 flex-1 truncate rounded-md border border-line-control px-2 py-1 text-sm text-muted-foreground" aria-labelledby="storage-settings-folder">
				{#if app.systemInfo}<PathText path={app.systemInfo.settingsFolder} />{:else}&mdash;{/if}
			</p>
			<Button variant="outline" class="shrink-0" onclick={openSettingsFolder}>Open</Button>
		</div>
	</SettingsRow>

	<SettingsRow advanced
		label="Skip the Recycle Bin"
		controlId="storage-skip-bin"
		description="Delete clips for good instead of moving them to the Recycle Bin. Deleting from the player still offers Undo for 5 seconds; after that they cannot be brought back."
	>
		<Switch id="storage-skip-bin" bind:checked={draft.storage.skipRecycleBin} />
	</SettingsRow>

	<SettingsRow
		label="Storage limit"
		controlId="storage-limit"
		description="Keep the clip folder under a size by deleting the oldest clips, recordings and screenshots first. Checked after every save and once an hour."
	>
		<div class="flex flex-col gap-2">
			<Switch
				id="storage-limit"
				checked={draft.storage.limitGb > 0}
				onCheckedChange={(on) => (draft.storage.limitGb = on ? Math.max(50, Math.ceil(usageGb * 1.25)) : 0)}
			/>
			{#if draft.storage.limitGb > 0}
				<NumberField label="Storage limit in gigabytes" unit="GB" min={1} max={100000} bind:value={draft.storage.limitGb} />
			{/if}
			{#if usage}
				<p class="tnum text-xs text-subtle">
					The folder holds {formatBytes(usage.bytes)} in {usage.files.toLocaleString()} files right now.
				</p>
			{/if}
		</div>
	</SettingsRow>

	<SettingsRow advanced
		label="Keep free on the drive"
		controlId="storage-min-free"
		description="When the clips drive has less free than this, a recording or session stops (what was recorded is kept) and none starts until there is room. You get a notification either way."
	>
		<div class="flex flex-col gap-2">
			<Switch
				id="storage-min-free"
				checked={draft.storage.minFreeGb > 0}
				onCheckedChange={(on) => (draft.storage.minFreeGb = on ? 5 : 0)}
			/>
			{#if draft.storage.minFreeGb > 0}
				<NumberField label="Free space to keep, in gigabytes" unit="GB" min={1} max={10000} bind:value={draft.storage.minFreeGb} />
			{/if}
			{#if usage}
				<p class="tnum text-xs text-subtle">{formatBytes(usage.freeBytes)} free on that drive right now.</p>
			{/if}
		</div>
	</SettingsRow>

	{#if showAdvanced()}<ShrinkSettings {draft} />{/if}

	<SettingsRow label="Delete old clips" controlId="storage-age" description="Clips older than this go, whatever the folder's size.">
		<div class="flex flex-col gap-2">
			<Switch
				id="storage-age"
				checked={draft.storage.maxAgeDays > 0}
				onCheckedChange={(on) => (draft.storage.maxAgeDays = on ? 30 : 0)}
			/>
			{#if draft.storage.maxAgeDays > 0}
				<NumberField label="Delete clips older than, in days" unit="days" min={1} max={36500} bind:value={draft.storage.maxAgeDays} />
			{/if}
		</div>
	</SettingsRow>

	{#if draft.storage.limitGb > 0 || draft.storage.maxAgeDays > 0}
		<SettingsRow
			label="Keep clips in albums"
			controlId="storage-keep-albums"
			description="Never delete a clip that is in an album. Favourites are always kept, and so is anything saved in the last 10 minutes."
		>
			<Switch id="storage-keep-albums" bind:checked={draft.storage.keepAlbums} />
			{#snippet below()}
				<div class="flex flex-wrap items-center gap-3">
					<p class="min-w-0 flex-1 text-xs text-muted-foreground">
						{draft.storage.skipRecycleBin
							? 'Removed clips are deleted for good.'
							: 'Removed clips go to the Recycle Bin, so the space comes back once it is emptied. Skip the Recycle Bin below frees it at once.'}
					</p>
					<Button variant="outline" size="sm" onclick={cleanNow} disabled={cleaning || draftIsDirty()} title={draftIsDirty() ? 'Apply the settings first' : 'Apply the limits now'}>
						{#if cleaning}<Spinner data-icon="inline-start" />{/if}
						Clean up now
					</Button>
				</div>
			{/snippet}
		</SettingsRow>
	{/if}

	<SettingsRow advanced label="Preview" labelId="storage-preview" description="Where a clip from Valorant would be saved right now.">
		<p class="tnum selectable rounded-md bg-raised px-2 py-2 text-xs text-foreground" aria-labelledby="storage-preview"><PathText path={preview} /></p>
	</SettingsRow>
</SettingsSection>
