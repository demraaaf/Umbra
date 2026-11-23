<script lang="ts">
	import AppWindowIcon from '@lucide/svelte/icons/app-window';
	import MonitorIcon from '@lucide/svelte/icons/monitor';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import { api } from '$lib/api';
	import type { CaptureSource, CaptureTarget } from '$lib/api/types';
	import * as Dialog from '$lib/components/ui/dialog';
	import * as ToggleGroup from '$lib/components/ui/toggle-group';
	import { Button } from '$lib/components/ui/button';
	import { Spinner } from '$lib/components/ui/spinner';
	import { app } from '$lib/stores/app.svelte';
	import SourcePreview from '../SourcePreview.svelte';

	/**
	 * Before a recording: what to record, a display or an open app. What the replay buffer
	 * captures is picked to begin with; anything else is recorded next to it, so saving clips
	 * keeps working. The record hotkey skips this and records the capture at once.
	 */
	let { open = $bindable(false), onrecord }: { open: boolean; onrecord: (target: CaptureTarget) => void } = $props();

	let sources = $state<CaptureSource[]>([]);
	let loading = $state(false);
	let tab = $state<'displays' | 'apps'>('displays');
	let chosen = $state<CaptureTarget | null>(null);

	const key = (t: CaptureTarget) => (t.kind === 'monitor' ? `m:${t.device}` : `w:${t.exe}:${t.title}`);
	const displays = $derived(sources.filter((s) => s.target.kind === 'monitor'));
	// Games first, then the rest by title.
	const apps = $derived(
		sources
			.filter((s) => s.target.kind === 'window')
			.sort((a, b) => Number(b.isGame) - Number(a.isGame) || a.label.localeCompare(b.label))
	);
	const shown = $derived(tab === 'displays' ? displays : apps);
	const chosenSource = $derived(chosen ? sources.find((s) => key(s.target) === key(chosen!)) : undefined);

	async function load() {
		loading = true;
		try {
			sources = await api.listCaptureSources();
		} catch {
			sources = [];
		} finally {
			loading = false;
		}
		// Start on what the buffer captures (an empty device is the first, primary, screen).
		const current = app.settings?.capture;
		let match: CaptureSource | undefined;
		if (current?.kind === 'monitor' && !current.device) match = displays[0];
		else if (current) match = sources.find((s) => key(s.target) === key(current));
		chosen = (match || displays[0])?.target ?? null;
		tab = chosen?.kind === 'window' ? 'apps' : 'displays';
	}

	$effect(() => {
		if (open) void load();
	});

	function start() {
		if (!chosen) return;
		open = false;
		onrecord(chosen);
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Content class="flex max-h-[88vh] flex-col sm:max-w-2xl">
		<Dialog.Header>
			<Dialog.Title>What to record</Dialog.Title>
			<Dialog.Description>A whole display, or one app's window. Saving clips keeps working while it records.</Dialog.Description>
		</Dialog.Header>

		<div class="flex items-center gap-2">
			<ToggleGroup.Root
				type="single"
				variant="outline"
				value={tab}
				onValueChange={(v) => v && (tab = v as 'displays' | 'apps')}
				aria-label="Kind of source"
			>
				<ToggleGroup.Item value="displays" class="px-3">
					<MonitorIcon data-icon="inline-start" />
					Displays
					<span class="tnum text-xs text-subtle">{displays.length}</span>
				</ToggleGroup.Item>
				<ToggleGroup.Item value="apps" class="px-3">
					<AppWindowIcon data-icon="inline-start" />
					Open apps
					<span class="tnum text-xs text-subtle">{apps.length}</span>
				</ToggleGroup.Item>
			</ToggleGroup.Root>
			<Button variant="ghost" size="icon-sm" class="ml-auto" onclick={load} disabled={loading} aria-label="Look for apps again" title="Look for apps again">
				<RefreshCwIcon />
			</Button>
		</div>

		<div class="-mx-4 min-h-[22rem] flex-1 overflow-y-auto px-4 py-1">
			{#if loading && sources.length === 0}
				<p class="flex items-center gap-2 py-8 text-sm text-muted-foreground" role="status"><Spinner /> Looking for displays and apps</p>
			{:else if shown.length === 0}
				<p class="py-8 text-center text-sm text-subtle">{tab === 'apps' ? 'No open app windows were found.' : 'No display was found.'}</p>
			{:else}
				<div class="grid grid-cols-[repeat(auto-fill,minmax(180px,1fr))] gap-3" role="radiogroup" aria-label={tab === 'apps' ? 'Open apps' : 'Displays'}>
					{#each shown as source (key(source.target))}
						{@const selected = chosen !== null && key(chosen) === key(source.target)}
						<button
							type="button"
							role="radio"
							aria-checked={selected}
							class="choice flex min-w-0 flex-col overflow-hidden rounded-md border text-left"
							class:selected
							onclick={() => (chosen = source.target)}
							ondblclick={start}
						>
							<SourcePreview target={source.target} maxWidth={320} fit="contain" empty="No preview" class="aspect-video w-full bg-black" />
							<span class="flex min-w-0 flex-col gap-0.5 border-t border-line px-2.5 py-2">
								<span class="flex min-w-0 items-center gap-1.5">
									<span class="truncate text-sm font-medium text-foreground">{source.label}</span>
									{#if source.isGame}
										<span class="shrink-0 rounded-xs border border-line-strong px-1 text-xs text-muted-foreground">Game</span>
									{/if}
								</span>
								<span class="truncate text-xs text-subtle">{source.detail}</span>
							</span>
						</button>
					{/each}
				</div>
			{/if}
		</div>

		<Dialog.Footer class="items-center">
			<p class="mr-auto truncate text-xs text-subtle">
				{#if chosenSource}Records {chosenSource.label}{/if}
			</p>
			<Button variant="outline" onclick={() => (open = false)}>Cancel</Button>
			<Button onclick={start} disabled={!chosen}>Record</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>

<style>
	.choice {
		border-color: var(--line);
		background: var(--chrome);
		transition:
			border-color var(--dur-fast) var(--ease-out),
			transform var(--dur-fast) var(--ease-out);
	}
	.choice:hover {
		border-color: var(--line-strong);
	}
	.choice:active {
		transform: scale(0.98);
	}
	.choice.selected {
		border-color: var(--text);
		box-shadow: 0 0 0 1px var(--text);
	}
	.choice:focus-visible {
		outline: 2px solid var(--focus);
		outline-offset: 2px;
	}
</style>
