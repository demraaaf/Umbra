<script lang="ts">
	import { onMount } from 'svelte';
	import { api } from '$lib/api';
	import type { MicrophoneInfo } from '$lib/api/types';
	import * as Select from '$lib/components/ui/select';
	import { Switch } from '$lib/components/ui/switch';
	import { Label } from '$lib/components/ui/label';
	import { app, patchSettings } from '$lib/stores/app.svelte';
	import { EXPLAIN } from '$lib/format/explain';
	import { bindingLabels } from '$lib/format/hotkey';
	import Keys from '../Keys.svelte';
	import Term from '../Term.svelte';
	import PanelSection from './PanelSection.svelte';

	const DEFAULT_DEVICE = '__default__';

	let mics = $state<MicrophoneInfo[]>([]);
	onMount(() => {
		void api.listMicrophones().then((list) => (mics = list)).catch(() => (mics = []));
	});

	const mic = $derived(app.settings?.audio.mic);
	const live = $derived(app.status?.micLive ?? false);
	const recording = $derived(app.status?.state === 'recording');
	/** Why the microphone can't be opened; its track is silent meanwhile. */
	const problem = $derived(app.status?.audioProblems?.find((p) => p.track === 'Microphone')?.problem ?? null);

	const deviceLabel = $derived.by(() => {
		if (!mic) return '';
		if (mic.deviceId === null) {
			const d = mics.find((m) => m.isDefault);
			return d ? `Default (${d.name})` : 'Default device';
		}
		return mics.find((m) => m.id === mic.deviceId)?.name ?? 'Unavailable device';
	});

	const stateText = $derived.by(() => {
		if (!mic?.enabled) return 'Off';
		if (!recording) return 'Not recording';
		if (problem) return 'Not connected';
		if (mic.pushToTalk) return live ? 'Live' : 'Hold key to talk';
		return live ? 'Live' : 'Silent';
	});
</script>

<PanelSection id="panel-mic" title="Microphone">
	{#snippet action()}
		<span class="flex items-center gap-1 text-xs whitespace-nowrap {live ? 'text-foreground' : 'text-subtle'}" role="status" aria-live="polite">
			<span class="mic-dot" class:live aria-hidden="true"></span>
			{stateText}
		</span>
	{/snippet}
	{#if mic}
		<div class="flex items-center justify-between gap-2">
			<Label for="panel-mic-on" class="text-xs font-normal text-foreground">Record the microphone</Label>
			<Switch
				id="panel-mic-on"
				size="sm"
				checked={mic.enabled}
				onCheckedChange={(v) => patchSettings((s) => (s.audio.mic.enabled = v))}
			/>
		</div>
		<Select.Root
			type="single"
			value={mic.deviceId ?? DEFAULT_DEVICE}
			onValueChange={(v) => patchSettings((s) => (s.audio.mic.deviceId = v === DEFAULT_DEVICE ? null : v))}
			disabled={!mic.enabled}
		>
			<Select.Trigger size="sm" class="w-full" aria-label="Microphone device">
				<span class="truncate">{deviceLabel}</span>
			</Select.Trigger>
			<Select.Content>
				<Select.Item value={DEFAULT_DEVICE} label="Default device">Default device</Select.Item>
				{#each mics as m (m.id)}
					<Select.Item value={m.id} label={m.name}>{m.name}</Select.Item>
				{/each}
			</Select.Content>
		</Select.Root>
		{#if problem && mic.enabled && recording}
			<p class="text-xs text-danger" role="alert">{problem}</p>
		{/if}
		<div class="flex items-center justify-between gap-2 text-xs">
			<Term label="Push to talk" explain={EXPLAIN.pushToTalk} class="text-muted-foreground" />
			{#if mic.pushToTalk}
				<Keys all={bindingLabels(mic.pushToTalkKey)} label="Push to talk key" />
			{:else}
				<span class="text-subtle">Off</span>
			{/if}
		</div>
	{/if}
</PanelSection>

<style>
	.mic-dot {
		width: 8px;
		height: 8px;
		border-radius: 999px;
		border: 1px solid var(--text-subtle);
	}
	.mic-dot.live {
		background: var(--text);
		border-color: var(--text);
	}
</style>
