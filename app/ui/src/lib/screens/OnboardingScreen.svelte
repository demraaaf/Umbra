<script lang="ts">
	import FolderOpenIcon from '@lucide/svelte/icons/folder-open';
	import { toast } from 'svelte-sonner';
	import { api, errorMessage } from '$lib/api';
	import type { MonitorInfo, RecordingMode, Settings } from '$lib/api/types';
	import CheckIcon from '@lucide/svelte/icons/check';
	import BenchmarkDialog from '$lib/components/app/settings/BenchmarkDialog.svelte';
	import { CODEC_LABELS, formatBitrate } from '$lib/format/format';
	import { Button } from '$lib/components/ui/button';
	import { Spinner } from '$lib/components/ui/spinner';
	import { Switch } from '$lib/components/ui/switch';
	import * as ToggleGroup from '$lib/components/ui/toggle-group';
	import Keys from '$lib/components/app/Keys.svelte';
	import PathText from '$lib/components/app/PathText.svelte';
	import ChoiceList, { type Choice } from '$lib/components/app/settings/ChoiceList.svelte';
	import HotkeyRecorder from '$lib/components/app/settings/HotkeyRecorder.svelte';
	import { app, openSettingsAt, patchSettings } from '$lib/stores/app.svelte';
	import { resetAdvanced } from '$lib/stores/settingsNav.svelte';
	import { comboLabels } from '$lib/format/hotkey';

	/**
	 * The first start: where clips go and the few settings everyone should choose, then where
	 * to find the rest. Nothing is stored until the end, so closing Umbra halfway shows this again.
	 */

	type Step = 'welcome' | 'folder' | 'basics' | 'quality' | 'done';
	const STEPS: Step[] = ['welcome', 'folder', 'basics', 'quality', 'done'];
	const MODES: Choice[] = [
		{ value: 'games', label: 'While a game is running', detail: 'Starts and stops by itself with your games.' },
		{ value: 'always', label: 'Always', detail: 'Whenever Umbra is open, including the desktop.' },
		{ value: 'manual', label: 'Only when I turn it on', detail: 'From the panel on the right.' }
	];
	const LENGTHS = [15, 30, 60, 120];

	let step = $state<Step>('welcome');
	let choice = $state<Settings>(structuredClone($state.snapshot(app.settings!)));
	let finishing = $state(false);
	let picking = $state(false);

	const index = $derived(STEPS.indexOf(step));

	// Clip quality: the setup from Settings → Video, run here so nobody has to pick a codec.
	let setupOpen = $state(false);
	let tuned = $state(false);
	let monitors = $state<MonitorInfo[]>([]);
	api.listMonitors().then((list) => (monitors = list)).catch(() => {});
	const screen = $derived.by(() => {
		const device = choice.capture.kind === 'monitor' ? choice.capture.device : '';
		return monitors.find((m) => device && m.device === device) ?? monitors.find((m) => m.primary) ?? monitors[0];
	});
	const perGame = $derived(choice.storage.folderTemplate.trim() !== '');

	async function pick() {
		picking = true;
		try {
			const picked = await api.pickFolder();
			if (picked) choice.storage.folder = picked;
		} catch (error) {
			toast.error('Could not open the folder picker', { description: errorMessage(error) });
		} finally {
			picking = false;
		}
	}

	async function finish(then?: () => void) {
		if (finishing) return;
		finishing = true;
		const picked = $state.snapshot(choice);
		const ok = await patchSettings((s) => {
			s.storage.folder = picked.storage.folder;
			s.storage.folderTemplate = picked.storage.folderTemplate;
			s.recording.mode = picked.recording.mode;
			s.recording.launchAtStartup = picked.recording.launchAtStartup;
			s.clip.lengthSecs = picked.clip.lengthSecs;
			s.clip.hotkey = picked.clip.hotkey;
			s.audio.mic.enabled = picked.audio.mic.enabled;
			// The setup measured this PC, so the stock per-game presets take its result too.
			if (tuned) {
				s.video = picked.video;
				for (const p of s.presets) p.video = structuredClone(picked.video);
			}
		});
		if (ok) {
			try {
				await api.finishOnboarding();
				resetAdvanced();
				app.firstRun = false;
				then?.();
			} catch (error) {
				toast.error('Could not save the settings', { description: errorMessage(error) });
			}
		}
		finishing = false;
	}
</script>

<div class="onboarding flex min-h-0 flex-1 justify-center overflow-y-auto bg-canvas px-6">
	<div class="flex w-full max-w-[34rem] flex-col py-12">
		<div class="mb-8 grid grid-cols-5 gap-1" aria-hidden="true">
			{#each STEPS as s, i (s)}
				<span class="h-0.5 rounded-full {i <= index ? 'bg-foreground' : 'bg-line-strong'}"></span>
			{/each}
		</div>

		{#key step}
			<section class="page flex flex-col gap-6" aria-labelledby="onboarding-title">
				{#if step === 'welcome'}
					<svg class="size-12 text-foreground" viewBox="0 0 16 16" aria-hidden="true">
						<circle cx="8" cy="8" r="6.5" fill="none" stroke="currentColor" stroke-width="1" />
						<path d="M8 1.5a6.5 6.5 0 0 0 0 13z" fill="currentColor" />
					</svg>
					<div class="flex flex-col gap-3">
						<h1 id="onboarding-title" class="text-2xl font-semibold tracking-tight text-balance text-foreground">Welcome to Umbra</h1>
						<p class="text-sm leading-relaxed text-muted-foreground">
							Umbra keeps the last moments of your game in memory. When something happens, press a key and it is saved as
							a clip. It can also record whole sessions. A few quick choices and you are set.
						</p>
					</div>
				{:else if step === 'folder'}
					<div class="flex flex-col gap-2">
						<h1 id="onboarding-title" class="text-xl font-semibold tracking-tight text-foreground">Where should your clips go?</h1>
						<p class="text-sm text-muted-foreground">Clips, recordings and screenshots are saved here. Pick a drive with room to spare.</p>
					</div>
					<div class="flex flex-col gap-3">
						<div class="flex items-center gap-2">
							<p class="selectable min-h-8 min-w-0 flex-1 truncate rounded-md border border-line-control bg-panel px-3 py-1.5 text-sm" aria-label="Clips folder">
								<PathText path={choice.storage.folder} />
							</p>
							<Button variant="outline" onclick={pick} disabled={picking}>
								{#if picking}<Spinner data-icon="inline-start" />{:else}<FolderOpenIcon data-icon="inline-start" />{/if}
								Choose
							</Button>
						</div>
						<label class="flex items-center justify-between gap-4 rounded-md border border-line px-3 py-2.5" for="onboarding-per-game">
							<span class="flex flex-col gap-0.5">
								<span class="text-sm text-foreground">A folder for each game</span>
								<span class="text-xs text-muted-foreground">Valorant clips in a Valorant folder, and so on.</span>
							</span>
							<Switch
								id="onboarding-per-game"
								checked={perGame}
								onCheckedChange={(on) => (choice.storage.folderTemplate = on ? '{game}' : '')}
							/>
						</label>
					</div>
				{:else if step === 'basics'}
					<div class="flex flex-col gap-2">
						<h1 id="onboarding-title" class="text-xl font-semibold tracking-tight text-foreground">The basics</h1>
						<p class="text-sm text-muted-foreground">Everything else has a sensible default and can be changed in Settings.</p>
					</div>

					<div class="flex flex-col gap-2">
						<h2 id="onboarding-mode" class="text-sm font-medium text-foreground">When should Umbra be ready to clip?</h2>
						<ChoiceList
							name="onboarding-mode"
							labelledBy="onboarding-mode"
							choices={MODES}
							bind:value={() => choice.recording.mode, (v) => v && (choice.recording.mode = v as RecordingMode)}
						/>
					</div>

					<div class="grid gap-5 sm:grid-cols-2">
						<div class="flex flex-col gap-2">
							<h2 id="onboarding-length" class="text-sm font-medium text-foreground">Clip length</h2>
							<ToggleGroup.Root
								type="single"
								variant="outline"
								class="w-full"
								value={String(choice.clip.lengthSecs)}
								onValueChange={(v) => v && (choice.clip.lengthSecs = Number(v))}
								aria-labelledby="onboarding-length"
							>
								{#each LENGTHS as secs (secs)}
									<ToggleGroup.Item value={String(secs)} class="tnum flex-1">{secs < 60 ? `${secs} s` : `${secs / 60} min`}</ToggleGroup.Item>
								{/each}
							</ToggleGroup.Root>
						</div>
						<div class="flex flex-col gap-2">
							<h2 id="onboarding-hotkey" class="text-sm font-medium text-foreground">Save hotkey</h2>
							<HotkeyRecorder bind:value={choice.clip.hotkey} defaultValue="Alt+F10" labelledBy="onboarding-hotkey" />
						</div>
					</div>

					<div class="flex flex-col divide-y divide-line rounded-md border border-line">
						<label class="flex items-center justify-between gap-4 px-3 py-2.5" for="onboarding-mic">
							<span class="flex flex-col gap-0.5">
								<span class="text-sm text-foreground">Record my microphone</span>
								<span class="text-xs text-muted-foreground">On its own track, so it can be turned down or off later.</span>
							</span>
							<Switch id="onboarding-mic" bind:checked={choice.audio.mic.enabled} />
						</label>
						<label class="flex items-center justify-between gap-4 px-3 py-2.5" for="onboarding-startup">
							<span class="flex flex-col gap-0.5">
								<span class="text-sm text-foreground">Start with Windows</span>
								<span class="text-xs text-muted-foreground">Umbra waits in the tray, ready when you play.</span>
							</span>
							<Switch id="onboarding-startup" bind:checked={choice.recording.launchAtStartup} />
						</label>
					</div>
				{:else if step === 'quality'}
					<div class="flex flex-col gap-2">
						<h1 id="onboarding-title" class="text-xl font-semibold tracking-tight text-foreground">Clip quality</h1>
						<p class="text-sm text-muted-foreground">
							Answer four questions about what you play, and Umbra measures your graphics card to pick the best-looking
							settings that don't cost you frames. About 10 seconds.
						</p>
					</div>
					{#if tuned}
						<p class="tnum flex items-center gap-2 rounded-md border border-line px-3 py-2.5 text-sm text-foreground" role="status">
							<CheckIcon class="size-4 shrink-0 text-success" aria-hidden="true" />
							{choice.video.width ? `${choice.video.width}×${choice.video.height}` : 'Your screen’s size'} · {choice.video.fps} fps ·
							{CODEC_LABELS[choice.video.codec]} · {formatBitrate(choice.video.bitrateKbps)}
						</p>
					{/if}
					<Button variant={tuned ? 'outline' : 'default'} class="self-start" onclick={() => (setupOpen = true)}>
						{tuned ? 'Run it again' : 'Find the best settings'}
					</Button>
					<p class="text-xs text-muted-foreground">Skip it and Umbra uses its defaults. Settings → Video has the same setup.</p>
				{:else}
					<div class="flex flex-col gap-2">
						<h1 id="onboarding-title" class="text-xl font-semibold tracking-tight text-foreground">You are set</h1>
						<p class="text-sm text-muted-foreground">A few things worth knowing:</p>
					</div>
					<dl class="flex flex-col divide-y divide-line rounded-md border border-line text-sm">
						<div class="flex items-center justify-between gap-4 px-3 py-3">
							<dt class="text-muted-foreground">Save a clip</dt>
							<dd><Keys parts={comboLabels(choice.clip.hotkey)} label="Save hotkey" /></dd>
						</div>
						<div class="flex items-center justify-between gap-4 px-3 py-3">
							<dt class="text-muted-foreground">Start or stop a recording</dt>
							<dd><Keys parts={comboLabels(choice.recording.recordHotkey)} label="Record hotkey" /></dd>
						</div>
						<div class="flex flex-col gap-1 px-3 py-3">
							<dt class="text-foreground">Every control and hotkey</dt>
							<dd class="text-muted-foreground">Settings → Help lists them all, mouse buttons included.</dd>
						</div>
						{#if !tuned}
						<div class="flex flex-col gap-1 px-3 py-3">
							<dt class="text-foreground">The best quality for this PC</dt>
							<dd class="text-muted-foreground">Settings → Video → Find the best settings measures your graphics card in about 10 seconds.</dd>
						</div>
						{/if}
						{#if app.systemInfo}
							<div class="flex flex-col gap-1 px-3 py-3">
								<dt class="text-foreground">Your settings</dt>
								<dd class="text-muted-foreground">Kept in <span class="selectable text-foreground"><PathText path={app.systemInfo.settingsFolder} /></span>.</dd>
							</div>
						{/if}
					</dl>
				{/if}
			</section>
		{/key}

		<div class="mt-8 flex items-center gap-2">
			{#if index > 0}
				<Button variant="ghost" onclick={() => (step = STEPS[index - 1])} disabled={finishing}>Back</Button>
			{/if}
			<div class="ml-auto flex items-center gap-2">
				{#if step === 'done'}
					<Button variant="outline" onclick={() => finish(() => openSettingsAt('help'))} disabled={finishing}>Open the Help page</Button>
					<Button onclick={() => finish()} disabled={finishing}>
						{#if finishing}<Spinner data-icon="inline-start" />{/if}
						Start using Umbra
					</Button>
				{:else}
					<Button onclick={() => (step = STEPS[index + 1])} disabled={step === 'folder' && !choice.storage.folder.trim()}>
						{step === 'welcome' ? 'Set up Umbra' : step === 'quality' && !tuned ? 'Skip' : 'Next'}
					</Button>
				{/if}
			</div>
		</div>
	</div>
</div>

<BenchmarkDialog
	bind:open={setupOpen}
	video={choice.video}
	windowCapture={choice.capture.kind === 'window'}
	{screen}
	onapply={async (video) => {
		choice.video = video;
		tuned = true;
	}}
/>

<style>
	.page {
		animation: enter 200ms var(--ease-out);
	}
	@keyframes enter {
		from {
			opacity: 0;
			transform: translateY(6px);
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.page {
			animation: none;
		}
	}
</style>
