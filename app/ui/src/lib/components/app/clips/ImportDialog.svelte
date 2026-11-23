<script lang="ts">
	import FileVideoIcon from '@lucide/svelte/icons/file-video';
	import FolderOpenIcon from '@lucide/svelte/icons/folder-open';
	import { api, errorMessage } from '$lib/api';
	import type { ImportFile, ImportSource } from '$lib/api/types';
	import * as Dialog from '$lib/components/ui/dialog';
	import * as ToggleGroup from '$lib/components/ui/toggle-group';
	import { Button } from '$lib/components/ui/button';
	import { Checkbox } from '$lib/components/ui/checkbox';
	import { Spinner } from '$lib/components/ui/spinner';
	import { formatBytes } from '$lib/format/format';
	import { importer, initImporter, startImport } from '$lib/stores/importer.svelte';
	import ImportProgressView from './ImportProgressView.svelte';
	import { untrack } from 'svelte';

	/**
	 * Brings clips from other recorders or any folder into the library: the recorders' folders
	 * found on this PC to tick, or a folder or files to choose. What would come in is listed by
	 * game before anything is copied; the originals stay where they are.
	 */
	let sources = $state<ImportSource[]>([]);
	/** Chosen folders and files, by path, with a name to show. */
	let picked = $state<{ path: string; label: string; detail: string }[]>([]);
	let checked = $state<string[]>([]);
	let files = $state<ImportFile[]>([]);
	let scanning = $state(false);
	let loaded = $state(false);
	let failure = $state<string | null>(null);
	/** Remove each original once its copy is checked. */
	let moveOriginals = $state(false);
	let scanToken = 0;

	$effect(() => {
		if (!importer.dialogOpen) return;
		void initImporter();
		if (loaded) return;
		loaded = true;
		api.listImportSources()
			.then((s) => (sources = s))
			.catch(() => (sources = []));
	});

	const fresh = $derived(files.filter((f) => !f.duplicate));
	const freshBytes = $derived(fresh.reduce((sum, f) => sum + f.bytes, 0));
	const byGame = $derived.by(() => {
		const games = new Map<string, number>();
		for (const f of fresh) games.set(f.game, (games.get(f.game) ?? 0) + 1);
		return [...games].sort((a, b) => b[1] - a[1]);
	});

	async function rescan() {
		const token = ++scanToken;
		failure = null;
		if (checked.length === 0) {
			files = [];
			return;
		}
		scanning = true;
		try {
			const found = await api.scanImport(checked);
			if (token === scanToken) files = found;
		} catch (error) {
			if (token === scanToken) failure = errorMessage(error);
		} finally {
			if (token === scanToken) scanning = false;
		}
	}

	function toggle(path: string, on: boolean) {
		checked = on ? [...checked, path] : checked.filter((p) => p !== path);
		void rescan();
	}

	async function chooseFolder() {
		const folder = await api.pickFolder();
		if (!folder || picked.some((p) => p.path === folder)) return;
		picked = [...picked, { path: folder, label: folder.split(/[\\/]/).pop() || folder, detail: folder }];
		toggle(folder, true);
	}

	async function chooseFiles() {
		addPaths(await api.pickVideos());
	}

	/** Chosen or dropped files and folders, ticked and looked through. */
	function addPaths(paths: string[]) {
		const added = paths.filter((p) => !picked.some((x) => x.path === p));
		picked = [...picked, ...added.map((p) => ({ path: p, label: p.split(/[\\/]/).pop() || p, detail: p }))];
		const ticked = paths.filter((p) => !checked.includes(p));
		if (ticked.length === 0) return;
		checked = [...checked, ...ticked];
		void rescan();
	}

	// Files dropped on the window come in here.
	$effect(() => {
		if (!importer.dialogOpen || importer.dropped.length === 0 || importer.running) return;
		const paths = importer.dropped;
		importer.dropped = [];
		untrack(() => addPaths(paths));
	});

	async function begin() {
		await startImport(files, moveOriginals);
		files = [];
		checked = [];
	}
</script>

<Dialog.Root bind:open={importer.dialogOpen}>
	<Dialog.Content class="flex max-h-[88vh] flex-col sm:max-w-lg">
		<Dialog.Header>
			<Dialog.Title>Import clips</Dialog.Title>
			<Dialog.Description>
				From other recorders or any folder. Clips are copied into your clips folder by game, several at once.
			</Dialog.Description>
		</Dialog.Header>

		<div class="-mx-4 flex min-h-0 flex-col gap-4 overflow-y-auto px-4 pb-1">
			{#if importer.running && importer.progress}
				<ImportProgressView progress={importer.progress} />
			{:else}
				<section class="flex flex-col gap-2" aria-labelledby="import-from">
					<h3 id="import-from" class="text-xs font-medium text-muted-foreground">From</h3>
					{#if sources.length === 0 && picked.length === 0}
						<p class="text-xs text-subtle">No other recorder's clips found in Videos. Choose a folder or files.</p>
					{/if}
					<ul class="flex flex-col divide-y divide-line rounded-md border border-line-control empty:hidden">
						{#each sources as source (source.path)}
							<li>
								<label class="flex cursor-pointer items-center gap-3 px-3 py-2">
									<Checkbox checked={checked.includes(source.path)} onCheckedChange={(on) => toggle(source.path, on === true)} />
									<span class="flex min-w-0 flex-1 flex-col">
										<span class="text-sm text-foreground">{source.label}</span>
										<span class="truncate text-xs text-subtle" title={source.path}>{source.path}</span>
									</span>
									<span class="tnum shrink-0 text-xs text-muted-foreground">{source.files} · {formatBytes(source.bytes)}</span>
								</label>
							</li>
						{/each}
						{#each picked as item (item.path)}
							<li>
								<label class="flex cursor-pointer items-center gap-3 px-3 py-2">
									<Checkbox checked={checked.includes(item.path)} onCheckedChange={(on) => toggle(item.path, on === true)} />
									<span class="flex min-w-0 flex-1 flex-col">
										<span class="truncate text-sm text-foreground">{item.label}</span>
										<span class="truncate text-xs text-subtle" title={item.detail}>{item.detail}</span>
									</span>
								</label>
							</li>
						{/each}
					</ul>
					<div class="flex flex-wrap gap-2">
						<Button variant="outline" size="sm" onclick={chooseFolder}>
							<FolderOpenIcon data-icon="inline-start" />
							Choose folder
						</Button>
						<Button variant="outline" size="sm" onclick={chooseFiles}>
							<FileVideoIcon data-icon="inline-start" />
							Choose files
						</Button>
					</div>
				</section>

				{#if scanning}
					<p class="flex items-center gap-2 text-xs text-muted-foreground" role="status"><Spinner /> Looking for clips</p>
				{:else if failure}
					<p class="text-xs text-destructive" role="alert">{failure}</p>
				{:else if checked.length > 0}
					<section class="flex flex-col gap-2" aria-labelledby="import-what">
						<h3 id="import-what" class="text-xs font-medium text-muted-foreground">
							<span class="tnum">{fresh.length} {fresh.length === 1 ? 'clip' : 'clips'} · {formatBytes(freshBytes)}</span>
							{#if files.length > fresh.length}<span class="font-normal text-subtle"> · {files.length - fresh.length} already in Umbra, left out</span>{/if}
						</h3>
						{#if byGame.length > 0}
							<ul class="grid grid-cols-[minmax(0,1fr)_auto] gap-x-4 gap-y-1 text-xs" aria-label="By game">
								{#each byGame as [game, count] (game)}
									<li class="contents">
										<span class="truncate text-foreground">{game}</span>
										<span class="tnum text-right text-muted-foreground">{count}</span>
									</li>
								{/each}
							</ul>
						{/if}
						<p class="text-xs text-subtle">Each keeps its date for sorting. MKV, MOV and WebM become MP4.</p>
					</section>

					<section class="flex flex-col gap-2" aria-labelledby="import-how">
						<h3 id="import-how" class="text-xs font-medium text-muted-foreground">Originals</h3>
						<ToggleGroup.Root
							type="single"
							variant="outline"
							class="w-full"
							value={moveOriginals ? 'move' : 'copy'}
							onValueChange={(v) => v && (moveOriginals = v === 'move')}
							aria-labelledby="import-how"
						>
							<ToggleGroup.Item value="copy" class="flex-1 px-2">Keep them</ToggleGroup.Item>
							<ToggleGroup.Item value="move" class="flex-1 px-2">Remove after copying</ToggleGroup.Item>
						</ToggleGroup.Root>
						<p class="text-xs text-subtle">
							{#if moveOriginals}
								Each original goes only once its copy is checked: as long, the same size, and every frame and sound there. They go to
								the Recycle Bin unless Storage says to skip it. One that does not check out stays where it is.
							{:else}
								The originals stay where they are.
							{/if}
						</p>
					</section>
				{/if}
			{/if}
		</div>

		<Dialog.Footer>
			{#if importer.running}
				<Button onclick={() => (importer.dialogOpen = false)}>Close</Button>
			{:else}
				<Button variant="outline" onclick={() => (importer.dialogOpen = false)}>Cancel</Button>
				<Button onclick={begin} disabled={fresh.length === 0 || scanning}>
					{moveOriginals ? 'Move' : 'Import'} {fresh.length > 0 ? fresh.length : ''}
				</Button>
			{/if}
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
