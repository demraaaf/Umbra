<script lang="ts">
	import { exportLabel, exportSummary } from '$lib/editor/exportPresets';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import FileOutputIcon from '@lucide/svelte/icons/file-output';
	import SlidersHorizontalIcon from '@lucide/svelte/icons/sliders-horizontal';
	import { Kbd } from '$lib/components/ui/kbd';
	import ExportSettingsDialog from './ExportSettingsDialog.svelte';
	import { Button } from '$lib/components/ui/button';
	import { Label } from '$lib/components/ui/label';
	import { Progress } from '$lib/components/ui/progress';
	import { Spinner } from '$lib/components/ui/spinner';
	import { Switch } from '$lib/components/ui/switch';
	import * as AlertDialog from '$lib/components/ui/alert-dialog';
	import * as ToggleGroup from '$lib/components/ui/toggle-group';
	import TrackVolume from './TrackVolume.svelte';
	import NumberField from '$lib/components/app/fields/NumberField.svelte';
	import TimecodeInput from '$lib/components/app/fields/TimecodeInput.svelte';
	import { ASPECTS, type AspectKey } from '$lib/editor/crop';
	import { speedLabel, type EditSession } from '$lib/editor/session.svelte';
	import type { Sequence } from '$lib/editor/sequence.svelte';
	import { openClip, revealClip } from '$lib/stores/app.svelte';
	import { clipTitle, formatResolution, formatTimecode } from '$lib/format/format';
	import MediaPool from './MediaPool.svelte';
	import OverlayInspector from './OverlayInspector.svelte';

	/**
	 * `session` is the clip being edited; `main` the first clip of the edit, which holds the
	 * export settings and fades of the whole; `sequence` the clips in the edit.
	 */
	let { session, main, sequence }: { session: EditSession; main: EditSession; sequence: Sequence } = $props();

	const clip = $derived(session.clip);
	const tracks = $derived(clip.audioTracks);
	const percent = $derived(Math.round(main.exportFraction * 100));
	const splitCount = $derived(session.innerSplits.length);
	const count = $derived(sequence.entries.length);
	const busy = $derived(main.busy);
	const canExport = $derived(count > 1 ? !busy && sequence.totalSecs > 0 : main.canExport);
	const info = $derived(main.exportInfoFor(sequence.totalSecs, count));

	let tab = $state<'edit' | 'media'>('edit');
	let confirmOverwrite = $state(false);
	let settingsOpen = $state(false);

	const FADE_SECS = [0, 0.5, 1, 2];
	const BETWEEN_SECS = [0, 0.3, 0.5, 1];
	const secsLabel = (v: number) => (v === 0 ? 'Off' : `${v} s`);

	function startExport() {
		const more = sequence.entries.slice(1).map((e) => e.session.buildItem());
		if (main.overwrite && more.length === 0 && main.exportOptions.format === 'mp4') confirmOverwrite = true;
		else void main.export(more);
	}
</script>

<aside class="flex w-[264px] shrink-0 flex-col border-l border-line bg-panel max-[999px]:w-[240px]" aria-label="Edit options">
	<div class="border-b border-line p-2">
		<ToggleGroup.Root
			type="single"
			variant="outline"
			size="sm"
			class="w-full"
			value={tab}
			onValueChange={(v) => v && (tab = v as 'edit' | 'media')}
			aria-label="Edit or add clips"
		>
			<ToggleGroup.Item value="edit" class="flex-1">Edit</ToggleGroup.Item>
			<ToggleGroup.Item value="media" class="flex-1">Clips{count > 1 ? ` (${count})` : ''}</ToggleGroup.Item>
		</ToggleGroup.Root>
	</div>
	{#if tab === 'media'}
		<MediaPool {sequence} {busy} />
	{:else}
	<div class="min-h-0 flex-1 overflow-y-auto">
		<!-- Trim -->
		<section class="flex flex-col gap-2 border-b border-line p-3" aria-labelledby="sec-trim">
			<div class="flex items-center justify-between">
				<h3 id="sec-trim" class="text-xs font-medium text-muted-foreground">Trim</h3>
				{#if session.trimmed}
					<Button variant="ghost" size="xs" onclick={() => session.resetTrim()} disabled={session.busy}>Reset</Button>
				{/if}
			</div>
			<div class="grid grid-cols-2 gap-x-2 gap-y-1">
				<Label for="trim-start" class="text-xs text-muted-foreground">Start</Label>
				<Label for="trim-end" class="text-xs text-muted-foreground">End</Label>
				<TimecodeInput id="trim-start" label="Trim start" value={session.inSecs} disabled={session.busy} oncommit={(t) => session.setIn(t)} />
				<TimecodeInput id="trim-end" label="Trim end" value={session.outSecs} disabled={session.busy} oncommit={(t) => session.setOut(t)} />
			</div>
			<div class="flex items-center justify-between">
				<span class="text-xs text-muted-foreground">Length</span>
				<span class="tnum text-sm text-foreground" title={session.speeds.length ? 'After speed changes' : ''}>{formatTimecode(session.outputSecs)}</span>
			</div>
		</section>

		<!-- Cuts -->
		<section class="flex flex-col gap-2 border-b border-line p-3" aria-labelledby="sec-cuts">
			<div class="flex items-center justify-between">
				<h3 id="sec-cuts" class="text-xs font-medium text-muted-foreground">Cuts</h3>
				<div class="flex items-center">
					{#if splitCount > 0}
						<Button variant="ghost" size="xs" onclick={() => session.clearSplits()} disabled={session.busy}>Clear cut points</Button>
					{/if}
					{#if session.cuts.length > 1}
						<Button variant="ghost" size="xs" onclick={() => session.clearCuts()} disabled={session.busy}>Restore all</Button>
					{/if}
				</div>
			</div>
			{#if splitCount === 0 && session.cuts.length === 0}
				<p class="text-xs leading-relaxed text-subtle">
					Press <Kbd>C</Kbd> to cut at the playhead. <Kbd>Del</Kbd> then takes out the part you are in;
					<Kbd>Q</Kbd> and <Kbd>W</Kbd> remove everything before or after it.
				</p>
			{:else}
				{#if splitCount > 0}
					<p class="text-xs text-muted-foreground">
						{splitCount === 1 ? '1 cut point' : `${splitCount} cut points`}.
						{#if session.segment}<Kbd>Del</Kbd> takes out the highlighted part.{/if}
					</p>
				{/if}
				{#if session.cuts.length > 0}
					<ul class="flex flex-col gap-1" aria-label="Parts taken out">
						{#each session.cuts as cut, i (cut.start)}
							<li class="flex items-center justify-between gap-2">
								<span class="tnum text-xs text-foreground">{formatTimecode(cut.start)} to {formatTimecode(cut.end)}</span>
								<Button variant="ghost" size="xs" onclick={() => session.restoreCut(i)} disabled={session.busy}>Restore</Button>
							</li>
						{/each}
					</ul>
				{/if}
			{/if}
		</section>

		<!-- Speed -->
		<section class="flex flex-col gap-2 border-b border-line p-3" aria-labelledby="sec-speed">
			<div class="flex items-center justify-between">
				<h3 id="sec-speed" class="text-xs font-medium text-muted-foreground">Speed</h3>
				{#if session.speeds.length > 0}
					<Button variant="ghost" size="xs" onclick={() => session.clearSpeeds()} disabled={session.busy}>All normal</Button>
				{/if}
			</div>
			{#if session.speeds.length === 0}
				<p class="text-xs leading-relaxed text-subtle">
					Select a part (or stand between cut points), then <Kbd>S</Kbd> slow motion, <Kbd>F</Kbd> fast, <Kbd>N</Kbd> normal.
					Right-click the timeline for more speeds.
				</p>
			{:else}
				<ul class="flex flex-col gap-1" aria-label="Parts at another speed">
					{#each session.speeds as range (range.start)}
						<li class="flex items-center justify-between gap-2">
							<span class="tnum text-xs text-foreground">{formatTimecode(range.start)} to {formatTimecode(range.end)}</span>
							<span class="tnum ml-auto text-xs text-muted-foreground">{speedLabel(range.speed)}</span>
							<Button variant="ghost" size="xs" onclick={() => session.setSpeed(range, 1)} disabled={session.busy}>Normal</Button>
						</li>
					{/each}
				</ul>
			{/if}
		</section>

		<!-- Fades, for the whole edit -->
		<section class="flex flex-col gap-2 border-b border-line p-3" aria-labelledby="sec-fades">
			<h3 id="sec-fades" class="text-xs font-medium text-muted-foreground">Fades{count > 1 ? ' (whole edit)' : ''}</h3>
			<div class="grid grid-cols-[4.5rem_minmax(0,1fr)] items-center gap-x-2 gap-y-1.5">
				<span id="fade-in-label" class="text-xs text-muted-foreground">In</span>
				<ToggleGroup.Root type="single" variant="outline" size="sm" class="w-full" value={String(main.fades.fadeIn)} onValueChange={(v) => v && main.setFades({ fadeIn: Number(v) })} aria-labelledby="fade-in-label" disabled={busy}>
					{#each FADE_SECS as v (v)}<ToggleGroup.Item value={String(v)} class="tnum flex-1 px-1 text-xs">{secsLabel(v)}</ToggleGroup.Item>{/each}
				</ToggleGroup.Root>
				<span id="fade-out-label" class="text-xs text-muted-foreground">Out</span>
				<ToggleGroup.Root type="single" variant="outline" size="sm" class="w-full" value={String(main.fades.fadeOut)} onValueChange={(v) => v && main.setFades({ fadeOut: Number(v) })} aria-labelledby="fade-out-label" disabled={busy}>
					{#each FADE_SECS as v (v)}<ToggleGroup.Item value={String(v)} class="tnum flex-1 px-1 text-xs">{secsLabel(v)}</ToggleGroup.Item>{/each}
				</ToggleGroup.Root>
				{#if count > 1}
					<span id="fade-between-label" class="text-xs text-muted-foreground">Between</span>
					<ToggleGroup.Root type="single" variant="outline" size="sm" class="w-full" value={String(main.fades.between)} onValueChange={(v) => v && main.setFades({ between: Number(v) })} aria-labelledby="fade-between-label" disabled={busy}>
						{#each BETWEEN_SECS as v (v)}<ToggleGroup.Item value={String(v)} class="tnum flex-1 px-1 text-xs">{secsLabel(v)}</ToggleGroup.Item>{/each}
					</ToggleGroup.Root>
				{/if}
			</div>
			<p class="text-xs text-subtle">From and to black, with the sound.{count > 1 ? ' Between dips through black where one clip meets the next.' : ''}</p>
		</section>

		<!-- Crop -->
		<section class="flex flex-col gap-2 border-b border-line p-3" aria-labelledby="sec-crop">
			<div class="flex items-center justify-between">
				<h3 id="sec-crop" class="text-xs font-medium text-muted-foreground">Crop</h3>
				<Switch
					checked={session.cropEnabled}
					onCheckedChange={(v) => session.setCropEnabled(v)}
					disabled={session.busy}
					aria-labelledby="sec-crop"
				/>
			</div>
			{#if session.cropEnabled}
				<ToggleGroup.Root
					type="single"
					variant="outline"
					size="sm"
					value={session.aspect}
					onValueChange={(v) => v && session.setAspect(v as AspectKey)}
					aria-label="Aspect ratio"
					class="w-full"
					disabled={session.busy}
				>
					{#each ASPECTS as aspect (aspect.key)}
						<ToggleGroup.Item value={aspect.key} class="tnum flex-1 px-1">{aspect.label}</ToggleGroup.Item>
					{/each}
				</ToggleGroup.Root>

				<div class="grid grid-cols-[16px_1fr_16px_1fr] items-center gap-x-2 gap-y-2">
					<Label for="crop-x" class="text-xs text-muted-foreground">X</Label>
					<NumberField id="crop-x" label="Crop left" unit="px" even min={0} max={clip.width} value={session.crop.x} disabled={session.busy} oncommit={(v) => session.setCropField('x', v)} />
					<Label for="crop-y" class="text-xs text-muted-foreground">Y</Label>
					<NumberField id="crop-y" label="Crop top" unit="px" even min={0} max={clip.height} value={session.crop.y} disabled={session.busy} oncommit={(v) => session.setCropField('y', v)} />
					<Label for="crop-w" class="text-xs text-muted-foreground">W</Label>
					<NumberField id="crop-w" label="Crop width" unit="px" even min={16} max={clip.width} value={session.crop.width} disabled={session.busy} oncommit={(v) => session.setCropField('width', v)} />
					<Label for="crop-h" class="text-xs text-muted-foreground">H</Label>
					<NumberField id="crop-h" label="Crop height" unit="px" even min={16} max={clip.height} value={session.crop.height} disabled={session.busy} oncommit={(v) => session.setCropField('height', v)} />
				</div>
				<div class="flex items-center justify-between">
					<p class="text-xs text-subtle">Source pixels, even values.</p>
					<Button variant="ghost" size="xs" onclick={() => session.resetCrop()} disabled={session.busy}>Reset</Button>
				</div>
			{:else}
				<p class="text-xs text-subtle">Off. The whole {formatResolution(clip.width, clip.height)} frame is kept.</p>
			{/if}
		</section>
		<OverlayInspector {session} />
		<!-- Audio tracks -->
		<section class="flex flex-col gap-2 p-3" aria-labelledby="sec-tracks">
			<div class="flex items-center justify-between">
				<h3 id="sec-tracks" class="text-xs font-medium text-muted-foreground">Audio tracks</h3>
				{#if session.audioChanged}
					<Button variant="ghost" size="xs" onclick={() => session.resetTracks()} disabled={session.busy}>Reset</Button>
				{/if}
			</div>
			{#if tracks.length === 0}
				<p class="text-xs text-subtle">This clip has no audio.</p>
			{:else}
				<ul class="flex flex-col divide-y divide-line rounded-md border border-line">
					{#each tracks as track (track.index)}
						<TrackVolume {session} {track} />
					{/each}
				</ul>
				<p class="text-xs text-subtle">
					100 % is the level as recorded; 0 % leaves the track out of the export.
				</p>
				{#if session.previewNote}
					<p class="text-xs text-muted-foreground" role="note">{session.previewNote}</p>
				{/if}
			{/if}
		</section>

	</div>
	{/if}

	<!-- Export -->
	<div class="flex shrink-0 flex-col gap-2 border-t border-line bg-panel p-3">
		<Button variant="outline" class="h-auto w-full justify-start gap-2 py-2" onclick={() => (settingsOpen = true)} disabled={busy}>
			<SlidersHorizontalIcon data-icon="inline-start" />
			<span>Export settings</span>
			<span class="ml-auto truncate text-xs font-normal text-muted-foreground">{exportLabel(main.exportOptions)}</span>
			<ChevronRightIcon class="text-subtle" />
		</Button>
		<p class="tnum text-xs leading-relaxed text-subtle" title="What the export will be">{exportSummary(info)}</p>

		<Button
			onclick={startExport}
			disabled={!canExport}
			class="w-full"
			title={canExport ? '' : 'Trim, cut, crop, change a speed or track first, or pick export settings that change the file'}
		>
			{#if busy}
				<Spinner data-icon="inline-start" />
				Exporting <span class="tnum">{percent}%</span>
			{:else}
				<FileOutputIcon data-icon="inline-start" />
				{main.overwrite && count === 1 && main.exportOptions.format === 'mp4' ? 'Export and replace' : count > 1 ? `Export ${count} clips as one` : 'Export'}
			{/if}
		</Button>

		{#if busy}
			<Progress value={percent} max={100} aria-label="Export progress" />
		{:else if main.exportState === 'done' && main.exportedFile && !(main.overwrite && count === 1)}
			{@const done = main.exportedFile}
			<div class="flex items-center justify-between gap-2 text-xs">
				<span class="min-w-0 truncate text-muted-foreground" title={done.fileName}>Saved {done.fileName}</span>
				{#if done.clip}
					<Button variant="ghost" size="xs" onclick={() => done.clip && openClip(done.clip.path)}>Open</Button>
				{:else}
					<Button variant="ghost" size="xs" onclick={() => void revealClip(done.path)}>Show</Button>
				{/if}
			</div>
		{:else if main.exportState === 'error'}
			<p class="selectable text-xs text-danger" role="alert">{main.exportError}</p>
		{/if}
	</div>
</aside>

<ExportSettingsDialog session={main} clips={count} totalSecs={sequence.totalSecs} bind:open={settingsOpen} />

<AlertDialog.Root bind:open={confirmOverwrite}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>Replace the original?</AlertDialog.Title>
			<AlertDialog.Description class="selectable">
				{clip.fileName} will be overwritten with the edited version. The original cannot be recovered.
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>Cancel</AlertDialog.Cancel>
			<AlertDialog.Action variant="destructive" onclick={() => main.export()}>Replace</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
