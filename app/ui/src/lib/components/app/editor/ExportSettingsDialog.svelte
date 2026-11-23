<script lang="ts">
	import * as Dialog from '$lib/components/ui/dialog';
	import * as InputGroup from '$lib/components/ui/input-group';
	import * as Select from '$lib/components/ui/select';
	import * as ToggleGroup from '$lib/components/ui/toggle-group';
	import { Button } from '$lib/components/ui/button';
	import { Label } from '$lib/components/ui/label';
	import { Switch } from '$lib/components/ui/switch';
	import ChoiceList from '$lib/components/app/settings/ChoiceList.svelte';
	import NumberField from '$lib/components/app/fields/NumberField.svelte';
	import {
		AUDIO_FORMATS,
		DISCORD_SIZES_MB,
		DISCORD_TIER,
		EXPORT_PRESETS,
		FRAME_RATES,
		GIF_WIDTHS,
		HEIGHTS,
		VIDEO_FORMATS,
		exportSummary,
		isAudioOnly,
		type ExportPresetKey
	} from '$lib/editor/exportPresets';
	import type { EditSession } from '$lib/editor/session.svelte';
	import type { OutputFormat, VideoCodec } from '$lib/api/types';

	/**
	 * How the edited clip is written: what it is for (a preset or the original's settings),
	 * the audio, and the file. Changes apply at once and are kept for the next export.
	 */
	let {
		session,
		open = $bindable(false),
		clips = 1,
		totalSecs
	}: { session: EditSession; open: boolean; clips?: number; totalSecs?: number } = $props();

	const clip = $derived(session.clip);
	const opts = $derived(session.exportOptions);
	const custom = $derived(opts.custom);
	const stem = $derived(clip.fileName.replace(/\.mp4$/i, ''));
	const info = $derived(session.exportInfoFor(totalSecs ?? session.outputSecs, clips));
	const audioOnly = $derived(isAudioOnly(opts.format));
	const gif = $derived(opts.format === 'gif');
	/** Replacing the original only makes sense for an MP4 of this one clip. */
	const canReplace = $derived(clips === 1 && opts.format === 'mp4');
	const formatDetail = $derived([...VIDEO_FORMATS, ...AUDIO_FORMATS].find((f) => f.value === opts.format)?.detail ?? '');
	const picture = $derived(session.cropActive ? session.crop : { width: clip.width, height: clip.height });

	const CODECS: { value: VideoCodec; label: string; detail: string }[] = [
		{ value: 'h264', label: 'H.264', detail: 'plays everywhere' },
		{ value: 'hevc', label: 'HEVC', detail: 'smaller, newer players' },
		{ value: 'av1', label: 'AV1', detail: 'smallest, newest players' }
	];
	const codecLabel = (value: string) =>
		value === 'same' ? `As recorded (${clip.codec.toUpperCase()})` : (CODECS.find((c) => c.value === value)?.label ?? value);
	const heightLabel = (h: number) => (h === 0 ? `As it is (${picture.width}×${picture.height})` : `${h}p`);
	const fpsLabel = (f: number) => (f === 0 ? `As recorded (${Math.round(clip.fps)} fps)` : `${f} fps`);
	// Only sizes and rates below the source make sense; nothing is scaled up.
	const heights = $derived(HEIGHTS.filter((h) => h === 0 || h < picture.height));
	const rates = $derived(FRAME_RATES.filter((f) => f === 0 || f < Math.round(clip.fps)));

	const presets = $derived(EXPORT_PRESETS.map((p) => ({ value: p.key, label: p.label, detail: p.detail })));
</script>

<Dialog.Root bind:open>
	<Dialog.Content class="flex max-h-[88vh] flex-col sm:max-w-lg">
		<Dialog.Header>
			<Dialog.Title>Export settings</Dialog.Title>
			<Dialog.Description>Changes apply right away and stay for your next exports.</Dialog.Description>
		</Dialog.Header>

		<div class="-mx-4 flex min-h-0 flex-col gap-5 overflow-y-auto px-4 pb-1">
			<!-- The kind of file -->
			<section class="flex flex-col gap-2" aria-labelledby="export-format">
				<h3 id="export-format" class="text-xs font-medium text-muted-foreground">Format</h3>
				<div class="grid grid-cols-[5.5rem_minmax(0,1fr)] items-center gap-x-3 gap-y-2">
					<span id="export-format-video" class="text-xs text-muted-foreground">Video</span>
					<ToggleGroup.Root
						type="single"
						variant="outline"
						size="sm"
						class="w-full"
						value={audioOnly ? '' : opts.format}
						onValueChange={(v) => v && session.setExport({ format: v as OutputFormat })}
						aria-labelledby="export-format-video"
					>
						{#each VIDEO_FORMATS as f (f.value)}<ToggleGroup.Item value={f.value} class="flex-1 px-1">{f.label}</ToggleGroup.Item>{/each}
					</ToggleGroup.Root>
					<span id="export-format-audio" class="text-xs text-muted-foreground">Audio only</span>
					<ToggleGroup.Root
						type="single"
						variant="outline"
						size="sm"
						class="w-full"
						value={audioOnly ? opts.format : ''}
						onValueChange={(v) => v && session.setExport({ format: v as OutputFormat })}
						aria-labelledby="export-format-audio"
					>
						{#each AUDIO_FORMATS as f (f.value)}<ToggleGroup.Item value={f.value} class="flex-1 px-1">{f.label}</ToggleGroup.Item>{/each}
					</ToggleGroup.Root>
				</div>
				<p class="text-xs text-subtle">
					{formatDetail}
					{#if opts.format !== 'mp4'}Saved next to the clip; the library shows MP4 files.{/if}
				</p>
				{#if gif}
					<div class="flex items-center gap-3">
						<span id="export-gif-width" class="w-[5.5rem] text-xs text-muted-foreground">Width</span>
						<ToggleGroup.Root
							type="single"
							variant="outline"
							size="sm"
							class="flex-1"
							value={String(opts.gifWidth)}
							onValueChange={(v) => v && session.setExport({ gifWidth: Number(v) })}
							aria-labelledby="export-gif-width"
						>
							{#each GIF_WIDTHS as w (w)}<ToggleGroup.Item value={String(w)} class="tnum flex-1 px-1">{w} px</ToggleGroup.Item>{/each}
						</ToggleGroup.Root>
					</div>
				{/if}
			</section>

			{#if !audioOnly && !gif}
			<!-- What it is for -->
			<section class="flex flex-col gap-2" aria-labelledby="export-for">
				<h3 id="export-for" class="text-xs font-medium text-muted-foreground">Made for</h3>
				<ChoiceList
					name="export-preset"
					labelledBy="export-for"
					choices={presets}
					bind:value={() => opts.preset, (v) => v && session.setExportPreset(v as ExportPresetKey)}
				/>

				{#if opts.preset === 'discord'}
					<div class="flex flex-col gap-1.5">
						<Label id="export-discord" class="text-xs text-muted-foreground">Upload limit</Label>
						<ToggleGroup.Root
							type="single"
							variant="outline"
							class="w-full"
							value={String(opts.discordMb)}
							onValueChange={(v) => v && session.setExport({ discordMb: Number(v) })}
							aria-labelledby="export-discord"
						>
							{#each DISCORD_SIZES_MB as mb (mb)}
								<ToggleGroup.Item value={String(mb)} class="tnum flex-1 px-2">
									{mb} MB <span class="text-xs text-subtle">{DISCORD_TIER[mb]}</span>
								</ToggleGroup.Item>
							{/each}
						</ToggleGroup.Root>
					</div>
				{/if}

				{#if opts.preset === 'custom'}
					<div class="grid grid-cols-[7rem_minmax(0,1fr)] items-center gap-x-3 gap-y-2">
						<Label id="export-codec" class="text-sm font-normal text-muted-foreground">Codec</Label>
						<Select.Root type="single" value={custom.codec} onValueChange={(v) => v && session.setCustomExport({ codec: v as VideoCodec | 'same' })}>
							<Select.Trigger class="w-full" aria-labelledby="export-codec">
								<span class="truncate">{codecLabel(custom.codec)}</span>
							</Select.Trigger>
							<Select.Content>
								<Select.Item value="same" label={codecLabel('same')}>{codecLabel('same')}</Select.Item>
								{#each CODECS as c (c.value)}
									<Select.Item value={c.value} label={c.label}>
										{c.label}
										<span class="ml-auto pl-3 text-xs text-subtle">{c.detail}</span>
									</Select.Item>
								{/each}
							</Select.Content>
						</Select.Root>

						<Label id="export-height" class="text-sm font-normal text-muted-foreground">Picture</Label>
						<Select.Root type="single" value={String(custom.height)} onValueChange={(v) => v && session.setCustomExport({ height: Number(v) })}>
							<Select.Trigger class="w-full" aria-labelledby="export-height">
								<span class="tnum truncate">{heightLabel(custom.height)}</span>
							</Select.Trigger>
							<Select.Content>
								{#each heights as h (h)}
									<Select.Item value={String(h)} label={heightLabel(h)}><span class="tnum">{heightLabel(h)}</span></Select.Item>
								{/each}
							</Select.Content>
						</Select.Root>

						<Label id="export-fps" class="text-sm font-normal text-muted-foreground">Frame rate</Label>
						<Select.Root type="single" value={String(custom.fps)} onValueChange={(v) => v && session.setCustomExport({ fps: Number(v) })}>
							<Select.Trigger class="w-full" aria-labelledby="export-fps">
								<span class="tnum truncate">{fpsLabel(custom.fps)}</span>
							</Select.Trigger>
							<Select.Content>
								{#each rates as f (f)}
									<Select.Item value={String(f)} label={fpsLabel(f)}><span class="tnum">{fpsLabel(f)}</span></Select.Item>
								{/each}
							</Select.Content>
						</Select.Root>

						<Label id="export-size" class="self-start pt-2 text-sm font-normal text-muted-foreground">File size</Label>
						<div class="flex flex-col gap-2">
							<ToggleGroup.Root
								type="single"
								variant="outline"
								class="w-full"
								value={custom.size}
								onValueChange={(v) => v && session.setCustomExport({ size: v as 'same' | 'bitrate' | 'limit' })}
								aria-labelledby="export-size"
							>
								<ToggleGroup.Item value="same" class="flex-1 px-2">Like now</ToggleGroup.Item>
								<ToggleGroup.Item value="bitrate" class="flex-1 px-2">Bitrate</ToggleGroup.Item>
								<ToggleGroup.Item value="limit" class="flex-1 px-2">Limit</ToggleGroup.Item>
							</ToggleGroup.Root>
							{#if custom.size === 'bitrate'}
								<NumberField label="Video bitrate" unit="kbit/s" min={500} max={200_000} step={1000} value={custom.kbps} oncommit={(v) => session.setCustomExport({ kbps: v })} />
							{:else if custom.size === 'limit'}
								<NumberField label="Size limit" unit="MB" min={1} max={4000} value={custom.limitMb} oncommit={(v) => session.setCustomExport({ limitMb: v })} />
							{/if}
						</div>
					</div>
				{/if}
			</section>

			{/if}

			{#if session.copyable && clips === 1}
			<!-- Cuts -->
			<section class="flex flex-col gap-2" aria-labelledby="export-cuts">
				<h3 id="export-cuts" class="text-xs font-medium text-muted-foreground">Cuts</h3>
				<ToggleGroup.Root
					type="single"
					variant="outline"
					class="w-full"
					value={opts.exactCuts ? 'exact' : 'fast'}
					onValueChange={(v) => v && session.setExport({ exactCuts: v === 'exact' })}
					aria-labelledby="export-cuts"
				>
					<ToggleGroup.Item value="fast" class="flex-1 px-2">Fast</ToggleGroup.Item>
					<ToggleGroup.Item value="exact" class="flex-1 px-2">Frame-exact</ToggleGroup.Item>
				</ToggleGroup.Root>
				<p class="text-xs text-subtle">
					{#if opts.exactCuts}
						Starts and cuts land on the exact frame, but the whole video is encoded again: for a long recording that takes about
						as long as the recording, or longer.
					{:else}
						The start and each cut move to the nearest keyframe (recordings have one every second or two) and nothing is encoded
						again: as fast as copying the file, with no loss.
					{/if}
				</p>
			</section>
			{/if}

			{#if !audioOnly && !gif}
			<!-- Audio -->
			<section class="flex flex-col gap-2" aria-labelledby="export-audio">
				<h3 id="export-audio" class="text-xs font-medium text-muted-foreground">Audio</h3>
				<ToggleGroup.Root
					type="single"
					variant="outline"
					class="w-full"
					value={session.exportForcesMix || clips > 1 ? 'mix' : opts.audio}
					onValueChange={(v) => v && session.setExport({ audio: v as 'tracks' | 'mix' })}
					disabled={session.exportForcesMix || clips > 1}
					aria-labelledby="export-audio"
				>
					<ToggleGroup.Item value="tracks" class="flex-1 px-2">Separate tracks</ToggleGroup.Item>
					<ToggleGroup.Item value="mix" class="flex-1 px-2">One mixed track</ToggleGroup.Item>
				</ToggleGroup.Root>
				<p class="text-xs text-subtle">
					{#if session.exportForcesMix || clips > 1}
						{opts.format !== 'mp4' || clips > 1
							? 'This export has one track: everything you kept, mixed, with your microphone.'
							: 'Sites and Discord play only the first track, so this preset mixes everything into one, with your microphone.'}
					{:else if opts.audio === 'mix'}
						Everything you kept in one track, with your microphone, for players that only play the first one.
					{:else}
						Each track you kept stays on its own, as recorded.
					{/if}
				</p>
			</section>
			{/if}

			<!-- File -->
			<section class="flex flex-col gap-3" aria-labelledby="export-file">
				<h3 id="export-file" class="text-xs font-medium text-muted-foreground">File</h3>
				<div class="flex flex-col gap-1.5">
					<Label for="export-name" class="text-sm font-normal text-muted-foreground">Name</Label>
					<InputGroup.Root data-disabled={(session.overwrite && canReplace) || undefined}>
						<InputGroup.Input
							id="export-name"
							bind:value={session.exportName}
							placeholder="{stem} (edit)"
							disabled={session.overwrite && canReplace}
							autocomplete="off"
							spellcheck={false}
						/>
						<InputGroup.Addon align="inline-end"><span class="text-subtle">.{opts.format}</span></InputGroup.Addon>
					</InputGroup.Root>
				</div>
				{#if canReplace}
					<div class="flex items-start justify-between gap-4">
						<Label for="export-replace" class="flex flex-col items-start gap-0.5 font-normal">
							<span class="text-sm text-foreground">Replace the original</span>
							<span class="text-xs text-muted-foreground">The original goes to the Recycle Bin, unless Storage says to skip it.</span>
						</Label>
						<Switch id="export-replace" bind:checked={session.overwrite} />
					</div>
				{/if}
				<div class="flex items-start justify-between gap-4">
					<Label for="export-copy" class="flex flex-col items-start gap-0.5 font-normal">
						<span class="text-sm text-foreground">Copy the file when it is done</span>
						<span class="text-xs text-muted-foreground">Then paste it into Discord, a chat or a folder with Ctrl+V.</span>
					</Label>
					<Switch id="export-copy" checked={opts.copyAfter} onCheckedChange={(v) => session.setExport({ copyAfter: v })} />
				</div>
			</section>
		</div>

		<Dialog.Footer class="items-center sm:justify-between">
			<p class="tnum text-xs text-muted-foreground" aria-live="polite">{exportSummary(info)}</p>
			<Button onclick={() => (open = false)}>Done</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
