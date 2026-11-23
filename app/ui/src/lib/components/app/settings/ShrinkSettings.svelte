<script lang="ts">
	import { onMount } from 'svelte';
	import { api } from '$lib/api';
	import type { Settings, ShrinkEstimate } from '$lib/api/types';
	import * as ToggleGroup from '$lib/components/ui/toggle-group';
	import { Button } from '$lib/components/ui/button';
	import { Spinner } from '$lib/components/ui/spinner';
	import { Switch } from '$lib/components/ui/switch';
	import { draftIsDirty } from '$lib/stores/app.svelte';
	import { formatBytes, plural } from '$lib/format/format';
	import NumberField from '../fields/NumberField.svelte';
	import SettingsRow from './SettingsRow.svelte';

	/**
	 * Making old recordings smaller in the background: encoded again as HEVC once they are
	 * old enough, on the graphics card, never while a game runs. Shows what it would save.
	 */
	let { draft }: { draft: Settings } = $props();

	const shrink = $derived(draft.storage.shrink);
	/** Bits per pixel per frame, as the backend uses them. */
	const BPP = { small: 0.035, balanced: 0.055, high: 0.085 } as const;
	const example = $derived(Math.round((2560 * 1440 * 60 * BPP[shrink.quality]) / 1e6));

	let estimate = $state<ShrinkEstimate | null>(null);
	let loading = $state(false);

	async function look() {
		loading = true;
		try {
			estimate = await api.shrinkEstimate();
		} catch {
			estimate = null;
		} finally {
			loading = false;
		}
	}

	onMount(() => {
		if (draft.storage.shrink.enabled) void look();
	});

	async function now() {
		await api.shrinkNow();
		setTimeout(look, 1500);
	}
</script>

<SettingsRow
	label="Compress old recordings"
	controlId="storage-shrink"
	description="Recordings take about 1 GB a minute. Once they are old enough, they are encoded again as HEVC at a lower bitrate in the background, on the graphics card, one at a time and never while a game runs or you record. Every audio track, the name and the date stay; albums and bookmarks keep working."
>
	<Switch id="storage-shrink" bind:checked={draft.storage.shrink.enabled} />
</SettingsRow>

{#if shrink.enabled}
	<SettingsRow label="Older than" controlId="storage-shrink-days">
		<NumberField id="storage-shrink-days" label="Compress recordings older than, in days" unit="days" min={0} max={3650} bind:value={draft.storage.shrink.afterDays} />
	</SettingsRow>

	<SettingsRow label="Clips too" controlId="storage-shrink-clips" description="Not only recordings: saved clips over 200 MB as well. Favourites are never compressed. Any clip can also be compressed by hand: right-click it, or select several.">
		<Switch id="storage-shrink-clips" bind:checked={draft.storage.shrink.includeClips} />
		{#snippet below()}
			<div class="flex flex-wrap items-center gap-3">
				<p class="tnum min-w-0 flex-1 text-xs text-muted-foreground" aria-live="polite">
					{#if draftIsDirty()}
						Apply the settings to see what it would do.
					{:else if loading}
						<Spinner class="mr-1 inline size-3" /> Looking through the clip folder
					{:else if estimate && estimate.files > 0}
						{plural(estimate.files, 'file')} to compress now: {formatBytes(estimate.bytes)}, about {formatBytes(estimate.afterBytes)} afterwards.
						{draft.storage.skipRecycleBin ? '' : 'Originals go to the Recycle Bin, so the space comes back once it is emptied.'}
						{#if estimate.running}One is being compressed now.{/if}
					{:else if estimate}
						Nothing to compress right now.
					{/if}
				</p>
				<Button variant="outline" size="sm" onclick={look} disabled={loading || draftIsDirty()}>Check</Button>
				<Button size="sm" onclick={now} disabled={draftIsDirty() || !estimate || estimate.files === 0}>Compress now</Button>
			</div>
		{/snippet}
	</SettingsRow>
{/if}

<SettingsRow
	label="Quality"
	labelId="storage-shrink-quality"
	description="For old recordings and for clips you compress by hand (right-click). A 1440p 60 fps recording becomes about {example} Mbit/s, against about 120 when recorded. Better looks closest to the original."
>
	<ToggleGroup.Root
		type="single"
		variant="outline"
		class="w-full"
		value={shrink.quality}
		onValueChange={(v) => v && (draft.storage.shrink.quality = v as 'small' | 'balanced' | 'high')}
		aria-labelledby="storage-shrink-quality"
	>
		<ToggleGroup.Item value="small" class="flex-1 px-2">Smaller</ToggleGroup.Item>
		<ToggleGroup.Item value="balanced" class="flex-1 px-2">Balanced</ToggleGroup.Item>
		<ToggleGroup.Item value="high" class="flex-1 px-2">Better</ToggleGroup.Item>
	</ToggleGroup.Root>
</SettingsRow>

<SettingsRow
	label="Frame rate"
	labelId="storage-shrink-fps"
	description="At 60 fps, a 144 fps recording becomes smaller still and is compressed in under half the time. Recordings at 60 fps or less keep theirs."
>
	<ToggleGroup.Root
		type="single"
		variant="outline"
		class="w-full"
		value={String(shrink.maxFps)}
		onValueChange={(v) => v && (draft.storage.shrink.maxFps = Number(v))}
		aria-labelledby="storage-shrink-fps"
	>
		<ToggleGroup.Item value="0" class="flex-1 px-2">Keep it</ToggleGroup.Item>
		<ToggleGroup.Item value="60" class="flex-1 px-2">60 fps</ToggleGroup.Item>
	</ToggleGroup.Root>
</SettingsRow>
