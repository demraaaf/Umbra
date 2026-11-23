<script lang="ts">
	import { onDestroy, tick, untrack } from 'svelte';
	import { toast } from 'svelte-sonner';
	import CrosshairIcon from '@lucide/svelte/icons/crosshair';
	import MountainIcon from '@lucide/svelte/icons/mountain';
	import ShuffleIcon from '@lucide/svelte/icons/shuffle';
	import HardDriveIcon from '@lucide/svelte/icons/hard-drive';
	import Share2Icon from '@lucide/svelte/icons/share-2';
	import { api, errorMessage } from '$lib/api';
	import type {
		BenchmarkAnswers,
		BenchmarkCost,
		BenchmarkReport,
		BenchmarkTrial,
		GameKind,
		MonitorInfo,
		Settings,
		VideoSettings
	} from '$lib/api/types';
	import * as Dialog from '$lib/components/ui/dialog';
	import * as RadioGroup from '$lib/components/ui/radio-group';
	import { Button } from '$lib/components/ui/button';
	import { Label } from '$lib/components/ui/label';
	import { Spinner } from '$lib/components/ui/spinner';
	import * as ToggleGroup from '$lib/components/ui/toggle-group';
	import { CODEC_LABELS, formatBitrate } from '$lib/format/format';
	import NumberField from '../fields/NumberField.svelte';

	/**
	 * The settings setup: four questions, a short encoder test, and the settings it recommends.
	 * Applying puts them in the draft and applies it; closing earlier changes nothing.
	 */
	let {
		open = $bindable(false),
		video,
		windowCapture,
		onapply,
		screen
	}: {
		open: boolean;
		/** The video settings being tuned: the default ones or a preset's. */
		video: VideoSettings;
		windowCapture: boolean;
		/** Puts the recommended settings in place and applies them. */
		onapply: (video: VideoSettings) => Promise<void>;
		screen: MonitorInfo | undefined;
	} = $props();

	type Step = 'game' | 'cost' | 'fixed' | 'share' | 'test' | 'result';
	const QUESTIONS: Step[] = ['game', 'cost', 'fixed', 'share'];

	let step = $state<Step>('game');
	let game = $state<GameKind>('shooter');
	let cost = $state<BenchmarkCost>('some');
	let sizeMode = $state<'screen' | 'fixed'>('screen');
	let fixedWidth = $state(1920);
	let fixedHeight = $state(1080);
	let fpsMode = $state<'auto' | 'fixed'>('auto');
	let fixedFps = $state(60);
	let stretched = $state<'no' | 'stretch' | 'fit'>('no');
	let share = $state<'keep' | 'share'>('keep');

	let running = $state(false);
	let label = $state('');
	let trials = $state<BenchmarkTrial[]>([]);
	let report = $state<BenchmarkReport | null>(null);
	let failure = $state<string | null>(null);
	let applying = $state(false);
	let useButton = $state<HTMLButtonElement | null>(null);

	const screenSize = $derived(screen ? `${screen.nativeWidth}×${screen.nativeHeight}` : 'your screen’s size');
	const questionIndex = $derived(QUESTIONS.indexOf(step));

	// Each opening starts over, unless a test is still running. Only `open` is tracked, so
	// a test finishing does not start over.
	$effect(() => {
		if (open && !untrack(() => running)) {
			step = 'game';
			report = null;
			failure = null;
			trials = [];
		}
	});

	let unlisten: (() => void) | null = null;
	onDestroy(() => unlisten?.());

	function answers(): BenchmarkAnswers {
		return {
			game,
			cost,
			size: sizeMode === 'fixed' ? [fixedWidth, fixedHeight] : null,
			fps: fpsMode === 'fixed' ? fixedFps : null,
			scaling: stretched === 'fit' ? 'fit' : stretched === 'stretch' ? 'stretch' : video.scaling,
			share: share === 'share'
		};
	}

	async function start() {
		step = 'test';
		running = true;
		failure = null;
		trials = [];
		label = 'Pausing recording';
		unlisten?.();
		unlisten = await api.onBenchmarkProgress((p) => {
			if (p.trial) trials = [...trials, p.trial];
			else label = p.label;
		});
		try {
			report = await api.runBenchmark(answers());
			step = 'result';
			// The Start button is gone; hand focus to the answer.
			await tick();
			useButton?.focus();
		} catch (error) {
			const message = errorMessage(error);
			failure = message === 'cancelled' ? null : message;
		} finally {
			running = false;
			unlisten?.();
			unlisten = null;
		}
	}

	function cancel() {
		void api.cancelBenchmark();
	}

	function onOpenChange(next: boolean) {
		if (!next && running) cancel();
	}

	async function use() {
		if (!report || applying) return;
		applying = true;
		try {
			await onapply(report.video);
			toast.success('New video settings applied');
			open = false;
		} catch (error) {
			toast.error('Could not apply the settings', { description: errorMessage(error) });
		} finally {
			applying = false;
		}
	}

	function next() {
		const i = QUESTIONS.indexOf(step);
		if (i < QUESTIONS.length - 1) step = QUESTIONS[i + 1];
		else step = 'test';
	}
	function back() {
		const i = QUESTIONS.indexOf(step);
		if (step === 'test' || step === 'result') step = 'share';
		else if (i > 0) step = QUESTIONS[i - 1];
	}

	// ---------- Result ----------

	function sizeText(v: VideoSettings): string {
		if (v.width !== null && v.height !== null) return `${v.width}×${v.height}`;
		return windowCapture ? 'Same as the window' : `Same as the screen (${screenSize})`;
	}
	function rateText(v: VideoSettings): string {
		if (v.rateMode === 'cqp') return `Constant QP ${v.qp}`;
		if (v.rateMode === 'cq') return `Constant quality ${v.cq}, peak ${formatBitrate(v.maxBitrateKbps)}`;
		if (v.rateMode === 'cbr') return `${formatBitrate(v.bitrateKbps)} constant`;
		return `${formatBitrate(v.bitrateKbps)}, peak ${formatBitrate(v.maxBitrateKbps)}`;
	}
	function extrasText(v: VideoSettings): string {
		const parts = [];
		if (v.spatialAq) parts.push('spatial AQ');
		if (v.multipass !== 'off') parts.push(`${v.multipass === 'quarter' ? 'quarter' : 'full'}-resolution first pass`);
		const text = parts.join(', ');
		return text ? text[0].toUpperCase() + text.slice(1) : 'None';
	}
	const rows = $derived.by(() => {
		if (!report) return [];
		const now = video;
		const v = report.video;
		const row = (label: string, value: string, before: string) => ({ label, value, before: before === value ? null : before });
		return [
			row('Codec', CODEC_LABELS[v.codec], CODEC_LABELS[now.codec]),
			row('Resolution', sizeText(v), sizeText(now)),
			row('Frame rate', `${v.fps} fps`, `${now.fps} fps`),
			row('Preset', `${v.preset} of 7`, `${now.preset} of 7`),
			row('Bitrate', rateText(v), rateText(now)),
			row('Extras', extrasText(v), extrasText(now)),
			row('Other shapes', v.scaling === 'fit' ? 'Black bars' : 'Stretch to fill', now.scaling === 'fit' ? 'Black bars' : 'Stretch to fill')
		];
	});

	function trialText(t: BenchmarkTrial): string {
		return `${CODEC_LABELS[t.codec]} · ${t.width}×${t.height} · ${t.fps} fps · preset ${t.preset}`;
	}
</script>

{#snippet option(value: string, title: string, detail: string, icon?: typeof CrosshairIcon, level?: number)}
	{@const Icon = icon}
	<Label for="bench-{value}" class="option flex cursor-pointer items-start gap-3 rounded-md border border-line px-3 py-2.5 font-normal">
		<RadioGroup.Item value={value} id="bench-{value}" class="mt-0.5" />
		<span class="flex min-w-0 flex-1 flex-col gap-0.5">
			<span class="text-sm font-medium text-foreground">{title}</span>
			<span class="text-xs text-muted-foreground">{detail}</span>
		</span>
		{#if Icon}
			<Icon class="mt-0.5 size-4 shrink-0 text-subtle" aria-hidden="true" />
		{:else if level}
			<span class="meter mt-1 flex shrink-0 items-end gap-0.5" aria-hidden="true">
				{#each [1, 2, 3] as bar (bar)}
					<span class="bar" class:on={bar <= level} style:height="{4 + bar * 3}px"></span>
				{/each}
			</span>
		{/if}
	</Label>
{/snippet}

<Dialog.Root bind:open {onOpenChange}>
	<Dialog.Content class="flex max-h-[88vh] flex-col sm:max-w-lg" showCloseButton={!running}>
		<Dialog.Header>
			<Dialog.Title>
				{#if step === 'test'}Testing the encoder{:else if step === 'result'}Recommended settings{:else}Find the best settings{/if}
			</Dialog.Title>
			<Dialog.Description>
				{#if questionIndex >= 0}
					Question {questionIndex + 1} of {QUESTIONS.length}
				{:else if step === 'test'}
					Recording pauses for the few seconds this takes.
				{:else}
					Measured on this PC. Nothing changes until you use them.
				{/if}
			</Dialog.Description>
			{#if questionIndex >= 0}
				<div class="steps mt-1 grid grid-cols-4 gap-1" aria-hidden="true">
					{#each QUESTIONS as q, i (q)}
						<span class="h-0.5 rounded-full {i <= questionIndex ? 'bg-foreground' : 'bg-line-strong'}"></span>
					{/each}
				</div>
			{/if}
		</Dialog.Header>

		<div class="-mx-4 min-h-0 overflow-y-auto px-4">
			{#if step === 'game'}
				<fieldset class="flex flex-col gap-2">
					<legend class="mb-2 text-sm text-foreground">What do you play most?</legend>
					<RadioGroup.Root bind:value={game} class="gap-2">
						{@render option('shooter', 'FPS games', 'CS2, Valorant, Apex, Fortnite. Lots of motion; smooth clips matter.', CrosshairIcon)}
						{@render option('story', 'Story and open world', 'Single-player and slower games. Detail matters more than frame rate.', MountainIcon)}
						{@render option('mixed', 'A bit of everything', 'A balance of the two.', ShuffleIcon)}
					</RadioGroup.Root>
				</fieldset>
			{:else if step === 'cost'}
				<fieldset class="flex flex-col gap-2">
					<legend class="mb-1 text-sm text-foreground">How much game performance may recording take?</legend>
					<p class="mb-2 text-xs text-muted-foreground">
						Recording uses the graphics card's video encoder and a little of its other work. More frames per second and
						slower, better-looking encoder settings take more.
					</p>
					<RadioGroup.Root bind:value={cost} class="gap-2">
						{@render option('least', 'Keep my frame rate', 'As light as possible. Clips still look good.', undefined, 1)}
						{@render option('some', 'A few fps for better clips', 'Noticeably sharper clips, and smoother ones in shooters.', undefined, 2)}
						{@render option('most', 'The best clips', 'My PC has frames to spare.', undefined, 3)}
					</RadioGroup.Root>
				</fieldset>
			{:else if step === 'fixed'}
				<div class="flex flex-col gap-5">
					<fieldset class="flex flex-col gap-2">
						<legend class="mb-2 text-sm text-foreground">Recording size</legend>
						<ToggleGroup.Root
			class="w-full" type="single" variant="outline" value={sizeMode} onValueChange={(v) => v && (sizeMode = v as 'screen' | 'fixed')} aria-label="Recording size">
							<ToggleGroup.Item value="screen" class="flex-1">My screen ({screenSize})</ToggleGroup.Item>
							<ToggleGroup.Item value="fixed" class="flex-1">A fixed size</ToggleGroup.Item>
						</ToggleGroup.Root>
						{#if sizeMode === 'fixed'}
							<div class="flex flex-wrap items-center gap-2">
								<NumberField label="Width" unit="px" min={16} max={16384} even bind:value={fixedWidth} class="w-32" />
								<span class="text-muted-foreground" aria-hidden="true">&times;</span>
								<NumberField label="Height" unit="px" min={16} max={16384} even bind:value={fixedHeight} class="w-32" />
								<div class="flex gap-1">
									{#each [[1920, 1080, '1080p'], [2560, 1440, '1440p'], [3840, 2160, '4K']] as [w, h, name] (name)}
										<Button
											variant="outline"
											size="xs"
											aria-pressed={fixedWidth === w && fixedHeight === h}
											onclick={() => {
												fixedWidth = w as number;
												fixedHeight = h as number;
											}}>{name}</Button
										>
									{/each}
								</div>
							</div>
						{:else}
							<p class="text-xs text-muted-foreground">If your screen's size is too much for the encoder, Umbra tries smaller sizes of the same shape.</p>
						{/if}
					</fieldset>

					<fieldset class="flex flex-col gap-2">
						<legend class="mb-2 text-sm text-foreground">Frame rate</legend>
						<ToggleGroup.Root
			class="w-full" type="single" variant="outline" value={fpsMode} onValueChange={(v) => v && (fpsMode = v as 'auto' | 'fixed')} aria-label="Frame rate">
							<ToggleGroup.Item value="auto" class="flex-1">Let Umbra choose</ToggleGroup.Item>
							<ToggleGroup.Item value="fixed" class="flex-1">A fixed rate</ToggleGroup.Item>
						</ToggleGroup.Root>
						{#if fpsMode === 'fixed'}
							<NumberField label="Frames per second" unit="fps" min={1} max={360} bind:value={fixedFps} class="w-40" />
						{/if}
					</fieldset>

					<fieldset class="flex flex-col gap-2">
						<legend class="mb-1 text-sm text-foreground">Do you play at a stretched resolution?</legend>
						<p class="mb-1 text-xs text-muted-foreground">Like 1440×1080 or 1280×960 on a 16:9 screen, common in CS2 and Valorant.</p>
						<RadioGroup.Root bind:value={stretched} class="gap-2">
							{@render option('no', 'No', 'Games run at the screen’s own shape.')}
							{@render option('stretch', 'Yes, stretched to fill the screen', 'Clips are stretched the same way, so they look like your game.')}
							{@render option('fit', 'Yes, with black bars', 'Clips keep the game’s shape with bars at the sides.')}
						</RadioGroup.Root>
					</fieldset>
				</div>
			{:else if step === 'share'}
				<fieldset class="flex flex-col gap-2">
					<legend class="mb-2 text-sm text-foreground">Where do your clips go?</legend>
					<RadioGroup.Root bind:value={share} class="gap-2">
						{@render option('keep', 'Mostly kept on this PC', 'The most efficient codec your graphics card has, for the best quality per megabyte.', HardDriveIcon)}
						{@render option('share', 'Discord and the web', 'H.264, which plays everywhere, including in Discord’s previews and every browser.', Share2Icon)}
					</RadioGroup.Root>
				</fieldset>
			{:else if step === 'test'}
				<div class="flex flex-col gap-3" aria-live="polite">
					{#if running}
						<p class="flex items-center gap-2 text-sm text-foreground"><Spinner /> {label}</p>
					{:else if failure}
						<p class="text-sm text-destructive" role="alert">{failure}</p>
					{:else}
						<p class="text-sm text-muted-foreground">
							Umbra encodes moving test frames at the settings that fit your answers, best quality first, until it finds
							one your encoder handles with room to spare. Recording pauses meanwhile and the replay buffer starts over.
						</p>
					{/if}
					{#if trials.length > 0}
						<ol class="trials flex flex-col text-xs">
							{#each trials as t, i (i)}
								<li class="trial grid grid-cols-[minmax(0,1fr)_auto_auto] items-center gap-3 border-t border-line py-1.5">
									<span class="truncate text-muted-foreground">{trialText(t)}</span>
									<span class="tnum text-foreground">{t.measuredFps === null ? '–' : `${Math.round(t.measuredFps)} fps`}</span>
									<span class="w-16 text-right {t.passed ? 'text-foreground' : 'text-subtle'}">{t.error ? 'Failed' : t.passed ? 'Fits' : 'Too slow'}</span>
								</li>
							{/each}
						</ol>
					{/if}
				</div>
			{:else if step === 'result' && report}
				<div class="flex flex-col gap-4">
					<dl class="grid grid-cols-[7rem_minmax(0,1fr)] gap-x-4 gap-y-2 text-sm">
						{#each rows as r (r.label)}
							<dt class="text-muted-foreground">{r.label}</dt>
							<dd class="min-w-0">
								<span class="tnum text-foreground">{r.value}</span>
								{#if r.before}
									<span class="block text-xs text-subtle">Now: {r.before}</span>
								{/if}
							</dd>
						{/each}
					</dl>
					<ul class="flex flex-col gap-1.5 text-xs text-muted-foreground">
						{#each report.notes as note (note)}
							<li>{note}</li>
						{/each}
					</ul>
					<details class="text-xs">
						<summary class="cursor-pointer text-muted-foreground select-none">What was measured ({report.trials.length})</summary>
						<ol class="mt-2 flex flex-col">
							{#each report.trials as t, i (i)}
								<li class="grid grid-cols-[minmax(0,1fr)_auto_auto] items-center gap-3 border-t border-line py-1.5">
									<span class="truncate text-muted-foreground">{trialText(t)}</span>
									<span class="tnum text-foreground">{t.measuredFps === null ? '–' : `${Math.round(t.measuredFps)} fps`}</span>
									<span class="w-16 text-right {t.passed ? 'text-foreground' : 'text-subtle'}">{t.error ? 'Failed' : t.passed ? 'Fits' : 'Too slow'}</span>
								</li>
							{/each}
						</ol>
					</details>
				</div>
			{/if}
		</div>

		<Dialog.Footer>
			{#if questionIndex > 0 || (step === 'test' && !running) || step === 'result'}
				<Button variant="ghost" class="sm:mr-auto" onclick={back}>Back</Button>
			{/if}
			{#if questionIndex >= 0}
				<Button onclick={next}>{questionIndex === QUESTIONS.length - 1 ? 'Continue' : 'Next'}</Button>
			{:else if step === 'test'}
				{#if running}
					<Button variant="outline" onclick={cancel}>Stop</Button>
				{:else}
					<Button onclick={start}>{failure ? 'Try again' : 'Start the test'}</Button>
				{/if}
			{:else if step === 'result'}
				<Button variant="outline" onclick={() => (open = false)}>Not now</Button>
				<Button bind:ref={useButton} onclick={use} disabled={applying}>
					{#if applying}<Spinner data-icon="inline-start" />{/if}
					Use these settings
				</Button>
			{/if}
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>

<style>
	:global(.option) {
		transition:
			background-color var(--dur-fast) var(--ease-out),
			border-color var(--dur-fast) var(--ease-out);
	}
	:global(.option:hover) {
		background: var(--surface-raised);
	}
	:global(.option:has([data-checked])),
	:global(.option:has([aria-checked='true'])) {
		background: var(--surface-selected);
		border-color: var(--line-control);
	}
	.bar {
		display: block;
		width: 4px;
		border-radius: 1px;
		background: var(--line-strong);
	}
	.bar.on {
		background: var(--text-muted);
	}
	.steps span {
		transition: background-color var(--dur-base) var(--ease-out);
	}
	.trial {
		animation: arrive 160ms var(--ease-out);
	}
	@keyframes arrive {
		from {
			opacity: 0;
			transform: translateY(-2px);
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.trial {
			animation: none;
		}
	}
	details summary:focus-visible {
		outline: 2px solid var(--focus);
		outline-offset: 2px;
		border-radius: 2px;
	}
</style>
