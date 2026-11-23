<script lang="ts">
	import FileIcon from '@lucide/svelte/icons/file';
	import FolderIcon from '@lucide/svelte/icons/folder';
	import FolderOpenIcon from '@lucide/svelte/icons/folder-open';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import SearchIcon from '@lucide/svelte/icons/search';
	import XIcon from '@lucide/svelte/icons/x';
	import { api, errorMessage } from '$lib/api';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import IconButton from '$lib/components/app/IconButton.svelte';

	/**
	 * A list of programs with a typed "Add" and optional one-click suggestions for programs not
	 * in the list yet, in rows by their `group` (`suggestLabel` for those without one). Entries are program names (`Game.exe`); with `paths`, also a program's
	 * full path, picked with Browse; with `folders`, also a folder, meaning every program in it.
	 */
	let {
		items = $bindable(),
		inputId,
		suggestions = [],
		names = {},
		emptyText,
		suggestLabel = 'Suggestions',
		paths = false,
		folders = false
	}: {
		items: string[];
		inputId: string;
		suggestions?: { exe: string; name: string; group?: string }[];
		/** Display names by executable, shown next to the file name. */
		names?: Record<string, string>;
		emptyText: string;
		suggestLabel?: string;
		paths?: boolean;
		folders?: boolean;
	} = $props();

	let draft = $state('');
	let note = $state('');
	let picking = $state(false);

	const isPath = (entry: string) => /[\\/]/.test(entry);
	const normal = (entry: string) => entry.replace(/\//g, '\\').replace(/\\+$/, '').toLowerCase();
	const has = (entry: string) => items.some((g) => normal(g) === normal(entry));
	const open = $derived(suggestions.filter((s) => !has(s.exe)));
	/** The suggestions not in the list yet, in rows by group, in the order the groups come. */
	const rows = $derived.by(() => {
		const out: { label: string; items: typeof open }[] = [];
		for (const s of open) {
			const label = s.group ?? suggestLabel;
			let row = out.find((r) => r.label === label);
			if (!row) out.push((row = { label, items: [] }));
			row.items.push(s);
		}
		return out;
	});

	/** The last part of a path: the program's file name, or the folder's name. */
	const leaf = (entry: string) => entry.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || entry;
	const isFolder = (entry: string) => isPath(entry) && !/\.exe$/i.test(entry.trim());

	function add(raw: string): boolean {
		let entry = raw.trim().replace(/^"(.*)"$/, '$1');
		if (!entry) return false;
		if (isPath(entry)) {
			entry = entry.replace(/\//g, '\\');
			if (!paths) {
				note = 'Use the program name only, for example Game.exe.';
				return false;
			}
			if (!/^([a-z]:[\\/]|\\\\)/i.test(entry)) {
				note = 'Use a full path, starting with the drive, for example C:\\Games\\Game.exe.';
				return false;
			}
			if (isFolder(entry) && !folders) {
				note = 'Use the path of the program itself, ending in .exe.';
				return false;
			}
		} else {
			if (/[:*?"<>|]/.test(entry)) {
				note = 'That is not a program name. Use one such as Game.exe.';
				return false;
			}
			if (!/\.exe$/i.test(entry)) entry += '.exe';
		}
		if (has(entry)) {
			note = `${leaf(entry)} is already in the list.`;
			return false;
		}
		items = [...items, entry];
		note = '';
		return true;
	}

	async function browse(kind: 'program' | 'folder') {
		picking = true;
		try {
			const picked = kind === 'program' ? await api.pickProgram() : await api.pickFolder();
			if (picked) add(picked);
		} catch (error) {
			note = `Could not open the picker: ${errorMessage(error)}`;
		} finally {
			picking = false;
		}
	}
</script>

<div class="flex flex-col gap-2">
	{#each rows as row (row.label)}
		<div class="flex flex-wrap items-center gap-1" role="group" aria-label={row.label}>
			<span class="mr-1 text-xs text-subtle">{row.label}</span>
			{#each row.items as s (s.exe)}
				<Button variant="outline" size="xs" onclick={() => add(s.exe)} title="Add {s.exe}" aria-label="Add {s.name}">
					<PlusIcon data-icon="inline-start" />
					{s.name}
				</Button>
			{/each}
		</div>
	{/each}

	{#if items.length > 0}
		<ul class="flex flex-col divide-y divide-line rounded-md border border-line-control">
			{#each items as entry (entry)}
				<li class="flex min-h-8 items-center gap-2 py-1 pr-1 pl-3">
					{#if isPath(entry)}
						{@const Icon = isFolder(entry) ? FolderIcon : FileIcon}
						<Icon class="size-3.5 shrink-0 text-subtle" aria-hidden="true" />
						<span class="flex min-w-0 flex-1 flex-col">
							<span class="selectable truncate text-sm">{isFolder(entry) ? `Everything in ${leaf(entry)}` : leaf(entry)}</span>
							<span class="selectable truncate text-xs text-subtle" title={entry}>{entry}</span>
						</span>
					{:else}
						<span class="selectable min-w-0 flex-1 truncate text-sm">{names[entry.toLowerCase()] ?? entry}</span>
						{#if names[entry.toLowerCase()]}<span class="truncate text-xs text-subtle">{entry}</span>{/if}
					{/if}
					<IconButton label="Remove {entry}" icon={XIcon} size="icon-xs" onclick={() => (items = items.filter((g) => g !== entry))} />
				</li>
			{/each}
		</ul>
	{:else}
		<p class="text-xs text-subtle">{emptyText}</p>
	{/if}

	<div class="flex items-center gap-2">
		<Input
			id={inputId}
			bind:value={draft}
			placeholder={paths ? 'Program.exe or a path' : 'Program.exe'}
			autocomplete="off"
			spellcheck={false}
			aria-describedby="{inputId}-note"
			onkeydown={(e) => {
				if (e.key === 'Enter') {
					e.preventDefault();
					if (add(draft)) draft = '';
				}
			}}
		/>
		<Button variant="outline" onclick={() => add(draft) && (draft = '')} disabled={!draft.trim()}>Add</Button>
	</div>
	{#if paths || folders}
		<div class="flex flex-wrap items-center gap-2">
			{#if paths}
				<Button variant="outline" size="sm" onclick={() => browse('program')} disabled={picking} title="Choose a program file">
					<SearchIcon data-icon="inline-start" />
					Choose program
				</Button>
			{/if}
			{#if folders}
				<Button variant="outline" size="sm" onclick={() => browse('folder')} disabled={picking} title="Choose a folder: every program inside it counts">
					<FolderOpenIcon data-icon="inline-start" />
					Choose folder
				</Button>
			{/if}
		</div>
	{/if}
	{#if note}
		<p id="{inputId}-note" class="text-xs text-foreground" role="status">{note}</p>
	{/if}
</div>
