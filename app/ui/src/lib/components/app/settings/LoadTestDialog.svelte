<script lang="ts">
	import { onDestroy, untrack } from 'svelte';
	import CheckIcon from '@lucide/svelte/icons/check';
	import { toast } from 'svelte-sonner';
	import { api, errorMessage } from '$lib/api';
	import type { LoadReport, LoadSuggestion, VideoSettings } from '$lib/api/types';
	import * as Dialog from '$lib/components/ui/dialog';
	import { Button } from '$lib/components/ui/button';
	import { Progress } from '$lib/components/ui/progress';
	import { Spinner } from '$lib/components/ui/spinner';
	import { app, applyDraft } from '$lib/stores/app.svelte';
	import { formatBytes } from '$lib/format/format';

	/**
	 * Recording under a game's load: a stand-in game runs alone, then while recording with the
	 * settings on the Video page. Shows what recording costs the game and what the load costs the
	 * recording, and offers changes.
	 */
	let {
		open = $bindable(false),
		presetId,
		presetName,
		onapply
	}: {
		open: boolean;
		/** The preset being tested, or null for the default video settings. */
		presetId: string | null;
		presetName: string;
		/** Puts changed video settings where they belong (the default ones or the preset's). */
		onapply: (video: VideoSettings) => void;
	} = $props();

	let running = $state(false);
	let label = $state('');
	let fraction = $state(0);
	let report = $state<LoadReport | null>(null);
	let failure = $state<string | null>(null);
	let applied = $state<number[]>([]);
	let unlisten: (() => void) | null = null;
	onDestroy(() => unlisten?.());

	// Opening starts over, unless a test is still running.
	$effect(() => {
		if (open && !untrack(() => running)) {
			report = null;
			failure = null;
			applied = [];
		}
	});

	async function start() {
		running = true;
		failure = null;
		report = null;
		fraction = 0;
		label = 'Pausing recording';
		unlisten?.();
		unlisten = await api.onLoadTestProgress((p) => {
			label = p.label;
			fraction = p.fraction;
		});
		try {
			report = await api.runLoadTest(presetId);
		} catch (error) {
			const message = errorMessage(error);
			failure = message === 'cancelled' ? null : message;
		} finally {
			running = false;
			unlisten?.();
			unlisten = null;
		}
	}

	function onOpenChange(next: boolean) {
		if (!next && running) void api.cancelBenchmark();
	}

	async function apply(suggestion: LoadSuggestion, index: number) {
		if (!app.draft) return;
		if (suggestion.video) onapply(suggestion.video);
		if (suggestion.clipSecs) app.draft.clip.lengthSecs = suggestion.clipSecs;
		try {
			await applyDraft();
			applied = [...applied, index];
			toast.success('Changed', { description: 'Run the test again to see the difference.' });
		} catch (error) {
			toast.error('Could not apply the change', { description: errorMessage(error) });
		}
	}

	const verdict = $derived.by(() => {
		if (!report) return null;
		if (report.dropPercent > 0.5) return { title: 'The recording drops frames under load', tone: 'bad' };
		if (report.fpsCost > 10 || report.lowCost > 15) return { title: 'Recording costs the game noticeably', tone: 'warn' };
		return { title: 'Recording is light on this PC', tone: 'good' };
	});

	const gb = (bytes: number) => formatBytes(bytes);
	const fps = (v: number) => `${Math.round(v)} fps`;
	const cost = (v: number) => (v < 0.5 ? 'no change' : `−${v.toFixed(v < 10 ? 1 : 0)} %`);
</script>

<Dialog.Root bind:open {onOpenChange}>
	<Dialog.Content class="flex max-h-[88vh] flex-col sm:max-w-lg" showCloseButton={!running}>
		<Dialog.Header>
			<Dialog.Title>{report && verdict ? verdict.title : 'Test under a game’s load'}</Dialog.Title>
			<Dialog.Description>
				{#if report}
					Measured with the {presetName} video settings and a stand-in game. Real games differ; treat this as a guide.
				{:else}
					Settings: {presetName}. About 25 seconds; recording pauses meanwhile.
				{/if}
			</Dialog.Description>
		</Dialog.Header>

		<div class="-mx-4 min-h-0 overflow-y-auto px-4">
			{#if running}
				<div class="flex flex-col gap-3 py-2" aria-live="polite">
					<p class="flex items-center gap-2 text-sm text-foreground"><Spinner /> {label}</p>
					<Progress value={Math.round(fraction * 100)} max={100} aria-label="Test progress" />
				</div>
			{:else if report}
				<div class="flex flex-col gap-4">
					<dl class="grid grid-cols-[9rem_minmax(0,1fr)] gap-x-4 gap-y-2 text-sm">
						<dt class="text-muted-foreground">Game frame rate</dt>
						<dd class="tnum">
							{fps(report.alone.fps)} → {fps(report.recording.fps)}
							<span class={report.fpsCost > 10 ? 'text-foreground' : 'text-subtle'}>({cost(report.fpsCost)})</span>
						</dd>
						<dt class="text-muted-foreground">1 % lows</dt>
						<dd class="tnum">
							{fps(report.alone.lowFps)} → {fps(report.recording.lowFps)}
							<span class={report.lowCost > 15 ? 'text-foreground' : 'text-subtle'}>({cost(report.lowCost)})</span>
						</dd>
						<dt class="text-muted-foreground">Recorded frames</dt>
						<dd class="tnum">
							{report.framesDropped === 0
								? `none dropped of ${report.framesRecorded.toLocaleString()}`
								: `${report.framesDropped.toLocaleString()} dropped of ${(report.framesRecorded + report.framesDropped).toLocaleString()} (${report.dropPercent.toFixed(1)} %)`}
						</dd>
						<dt class="text-muted-foreground">Memory</dt>
						<dd class="tnum">
							game {gb(report.gameMemory)} + replay buffer {gb(report.bufferMemory)} of {gb(report.totalMemory)}
						</dd>
					</dl>

					{#if report.suggestions.length > 0}
						<section class="flex flex-col gap-2" aria-label="Suggested changes">
							<h3 class="text-xs font-medium text-muted-foreground">Suggested changes, most effective first</h3>
							<ul class="flex flex-col gap-2">
								{#each report.suggestions as suggestion, i (i)}
									<li class="flex items-start gap-3 rounded-md border border-line px-3 py-2.5">
										<p class="min-w-0 flex-1 text-sm text-foreground">{suggestion.text}</p>
										{#if applied.includes(i)}
											<span class="flex shrink-0 items-center gap-1 pt-0.5 text-xs text-muted-foreground"><CheckIcon class="size-3.5" aria-hidden="true" /> Applied</span>
										{:else}
											<Button variant="outline" size="sm" class="shrink-0" onclick={() => apply(suggestion, i)}>Apply</Button>
										{/if}
									</li>
								{/each}
							</ul>
						</section>
					{:else}
						<p class="text-sm text-muted-foreground">Nothing to change: the game kept its frame rate and the recording dropped no frames.</p>
					{/if}
				</div>
			{:else}
				<div class="flex flex-col gap-3 text-sm text-muted-foreground">
					<p>
						Umbra plays a stand-in game: it keeps the graphics card fully busy at your screen's size, keeps half the
						processor's cores busy and holds a quarter of the memory (up to 8 GB), as a demanding game does.
					</p>
					<p>
						It measures the game's frame rate and 1 % lows on their own, then while recording with these settings, and
						counts the frames the recording drops. Then it suggests what to change, if anything.
					</p>
					{#if failure}
						<p class="text-destructive" role="alert">{failure}</p>
					{/if}
				</div>
			{/if}
		</div>

		<Dialog.Footer>
			{#if running}
				<Button variant="outline" onclick={() => void api.cancelBenchmark()}>Stop</Button>
			{:else if report}
				<Button variant="outline" onclick={start}>Test again</Button>
				<Button onclick={() => (open = false)}>Done</Button>
			{:else}
				<Button onclick={start}>{failure ? 'Try again' : 'Start the test'}</Button>
			{/if}
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
