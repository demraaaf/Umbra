<script lang="ts">
	import { onMount } from 'svelte';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import { api } from '$lib/api';
	import CheckIcon from '@lucide/svelte/icons/check';
	import SearchIcon from '@lucide/svelte/icons/search';
	import type { CaptureSource, MonitorInfo, Settings } from '$lib/api/types';
	import { Button } from '$lib/components/ui/button';
	import * as InputGroup from '$lib/components/ui/input-group';
	import { Spinner } from '$lib/components/ui/spinner';
	import { Switch } from '$lib/components/ui/switch';
	import * as ToggleGroup from '$lib/components/ui/toggle-group';
	import SourcePreview from '../SourcePreview.svelte';
	import ChoiceList, { type Choice } from './ChoiceList.svelte';
	import SettingsRow from './SettingsRow.svelte';
	import SettingsSection from './SettingsSection.svelte';

	let { draft }: { draft: Settings } = $props();

	let monitors = $state<MonitorInfo[]>([]);
	/** Open windows, as the capture switcher lists them (Umbra and the desktop left out). */
	let windows = $state<WindowSource[]>([]);
	let loadingMonitors = $state(true);
	let loadingWindows = $state(false);
	let query = $state('');

	const SEP = '\u0000';

	interface WindowSource {
		exe: string;
		title: string;
		isGame: boolean;
	}

	async function loadMonitors() {
		loadingMonitors = true;
		try {
			monitors = await api.listMonitors();
		} finally {
			loadingMonitors = false;
		}
	}

	async function loadWindows() {
		loadingWindows = true;
		try {
			const sources: CaptureSource[] = await api.listCaptureSources();
			windows = sources.flatMap((s) =>
				s.target.kind === 'window' ? [{ exe: s.target.exe, title: s.target.title, isGame: s.isGame }] : []
			);
		} finally {
			loadingWindows = false;
		}
	}

	onMount(() => {
		void loadMonitors();
		void loadWindows();
	});

	const primary = $derived(monitors.find((m) => m.primary) ?? monitors[0]);

	const monitorChoices = $derived<Choice[]>(
		monitors.map((m, i) => ({
			value: m.device,
			label: `Display ${i + 1}`,
			detail: `${m.device.replace('\\\\.\\', '')} · ${m.width}×${m.height} · ${m.refreshHz} Hz`,
			badge: m.primary ? 'Primary' : undefined
		}))
	);

	const selectedDevice = $derived.by(() => {
		if (draft.capture.kind !== 'monitor') return undefined;
		return draft.capture.device || primary?.device;
	});

	const selectedWindow = $derived(
		draft.capture.kind === 'window' ? `${draft.capture.exe}${SEP}${draft.capture.title}` : undefined
	);

	interface Tile extends WindowSource {
		key: string;
		/** The saved window, which is not open right now. */
		missing: boolean;
	}

	const tiles = $derived.by<Tile[]>(() => {
		const needle = query.trim().toLowerCase();
		const list: Tile[] = windows
			.filter((w) => !needle || `${w.exe} ${w.title}`.toLowerCase().includes(needle))
			.map((w) => ({ ...w, key: `${w.exe}${SEP}${w.title}`, missing: false }));
		// A saved target that is not open right now stays visible so it is not silently lost.
		if (draft.capture.kind === 'window' && draft.capture.exe) {
			const key = `${draft.capture.exe}${SEP}${draft.capture.title}`;
			if (!windows.some((w) => `${w.exe}${SEP}${w.title}` === key)) {
				list.unshift({ exe: draft.capture.exe, title: draft.capture.title, isGame: false, key, missing: true });
			}
		}
		return list;
	});
	const groups = $derived.by(() => {
		const games = tiles.filter((t) => t.isGame || t.missing);
		const others = tiles.filter((t) => !t.isGame && !t.missing);
		return [
			{ id: 'games', title: 'Games', list: games },
			{ id: 'others', title: games.length ? 'Other windows' : '', list: others }
		].filter((g) => g.list.length > 0);
	});

	function setKind(kind: string) {
		if (kind === 'monitor' && draft.capture.kind !== 'monitor') {
			draft.capture = { kind: 'monitor', device: primary?.device ?? '' };
		} else if (kind === 'window' && draft.capture.kind !== 'window') {
			const first = windows[0];
			draft.capture = { kind: 'window', exe: first?.exe ?? '', title: first?.title ?? '' };
		}
	}

	function setMonitor(device: string | undefined) {
		if (device !== undefined) draft.capture = { kind: 'monitor', device };
	}

	function setWindow(key: string | undefined) {
		if (!key) return;
		const [exe, title = ''] = key.split(SEP);
		draft.capture = { kind: 'window', exe, title };
	}
</script>

<SettingsSection id="capture" title="Capture" description="What gets recorded.">
	<SettingsRow label="Source" labelId="capture-source" description="Record a whole monitor, or a single window.">
		<ToggleGroup.Root
			type="single"
			variant="outline"
			value={draft.capture.kind}
			onValueChange={(v) => v && setKind(v)}
			aria-labelledby="capture-source"
		>
			<ToggleGroup.Item value="monitor" class="min-w-20">Monitor</ToggleGroup.Item>
			<ToggleGroup.Item value="window" class="min-w-20">Window</ToggleGroup.Item>
		</ToggleGroup.Root>
	</SettingsRow>

	{#if draft.capture.kind === 'monitor'}
		<SettingsRow label="Monitor" labelId="capture-monitor">
			{#if loadingMonitors && monitors.length === 0}
				<div class="flex h-8 items-center gap-2 text-xs text-subtle" role="status"><Spinner /> Looking for monitors</div>
			{:else}
				<ChoiceList
					name="monitor"
					labelledBy="capture-monitor"
					choices={monitorChoices}
					bind:value={() => selectedDevice, setMonitor}
				/>
			{/if}
		</SettingsRow>
	{:else}
		<SettingsRow
			label="Window"
			labelId="capture-window"
			description="Matched by program name first, then by title. The window must be open to be recorded."
		>
			<div class="flex items-center gap-2">
				<InputGroup.Root>
					<InputGroup.Addon><SearchIcon aria-hidden="true" /></InputGroup.Addon>
					<InputGroup.Input
						bind:value={query}
						type="search"
						placeholder="Find a window"
						aria-label="Find a window"
						autocomplete="off"
						class="[&::-webkit-search-cancel-button]:hidden"
					/>
				</InputGroup.Root>
				<Button variant="outline" size="icon" onclick={loadWindows} aria-label="Refresh the window list" title="Refresh the window list" disabled={loadingWindows}>
					{#if loadingWindows}<Spinner />{:else}<RefreshCwIcon />{/if}
				</Button>
			</div>
			{#snippet below()}
				{#if loadingWindows && windows.length === 0}
					<p class="flex h-24 items-center justify-center gap-2 text-xs text-subtle" role="status"><Spinner /> Looking for windows</p>
				{:else if tiles.length === 0}
					<p class="flex h-24 items-center justify-center rounded-md border border-dashed border-line-strong text-xs text-subtle">
						{query.trim() ? `No open window matches “${query.trim()}”.` : 'No windows are open.'}
					</p>
				{:else}
					<div class="flex flex-col gap-3" role="group" aria-labelledby="capture-window">
						{#each groups as group (group.id)}
							<div class="flex flex-col gap-1.5">
								{#if group.title}
									<h3 class="text-xs font-medium text-muted-foreground">{group.title}</h3>
								{/if}
								<div class="tiles grid gap-2">
									{#each group.list as tile (tile.key)}
										{@const chosen = selectedWindow === tile.key}
										<button
											type="button"
											class="tile flex min-w-0 flex-col overflow-hidden rounded-md border text-left"
											class:chosen
											aria-pressed={chosen}
											title={tile.missing ? `${tile.title || tile.exe} is not open right now` : `Record ${tile.title || tile.exe}`}
											onclick={() => setWindow(tile.key)}
										>
											<span class="relative block aspect-video w-full bg-black">
												{#if tile.missing}
													<span class="flex size-full items-center justify-center text-xs text-subtle">Not open right now</span>
												{:else}
													<SourcePreview
														target={{ kind: 'window', exe: tile.exe, title: tile.title }}
														maxWidth={320}
														fit="contain"
														empty="Minimised"
														class="absolute inset-0 bg-black"
													/>
												{/if}
												{#if chosen}
													<span class="absolute top-1.5 right-1.5 flex size-5 items-center justify-center rounded-full bg-foreground text-background">
														<CheckIcon class="size-3" strokeWidth={3} aria-hidden="true" />
													</span>
												{/if}
											</span>
											<span class="flex min-w-0 flex-col gap-0.5 border-t border-line px-2 py-1.5">
												<span class="truncate text-xs font-medium text-foreground">{tile.title || tile.exe}</span>
												<span class="truncate text-xs text-subtle">{tile.exe}</span>
											</span>
										</button>
									{/each}
								</div>
							</div>
						{/each}
					</div>
				{/if}
			{/snippet}
		</SettingsRow>
	{/if}

	<SettingsRow label="Show the mouse cursor" controlId="capture-cursor" description="Draw the cursor into the recording.">
		<Switch id="capture-cursor" bind:checked={draft.video.captureCursor} />
	</SettingsRow>

</SettingsSection>

<style>
	.tiles {
		grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
	}
	.tile {
		border-color: var(--line);
		background: var(--surface-panel);
		transition:
			border-color var(--dur-fast) var(--ease-out),
			background-color var(--dur-fast) var(--ease-out),
			transform var(--dur-fast) var(--ease-out);
	}
	.tile:hover {
		border-color: var(--line-control);
		background: var(--surface-raised);
	}
	.tile:active {
		transform: scale(0.98);
	}
	.tile.chosen {
		border-color: var(--text);
		box-shadow: 0 0 0 1px var(--text);
		background: var(--surface-selected);
	}
	.tile:focus-visible {
		outline: 2px solid var(--focus);
		outline-offset: 2px;
	}
</style>
