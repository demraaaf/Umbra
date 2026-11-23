<script lang="ts">
	import { onMount } from 'svelte';
	import { api } from '$lib/api';
	import type { AudioAppInfo, AudioMode, MicrophoneInfo, Settings } from '$lib/api/types';
	import { Button } from '$lib/components/ui/button';
	import * as Select from '$lib/components/ui/select';
	import { Switch } from '$lib/components/ui/switch';
	import * as ToggleGroup from '$lib/components/ui/toggle-group';
	import { AUDIO_BITRATE_PRESETS, EXPLAIN } from '$lib/format/explain';
	import NumberField from '../fields/NumberField.svelte';
	import ExeList from './ExeList.svelte';
	import HotkeyRecorder from './HotkeyRecorder.svelte';
	import SettingsRow from './SettingsRow.svelte';
	import SettingsSection from './SettingsSection.svelte';

	let { draft }: { draft: Settings } = $props();

	let playing = $state<AudioAppInfo[]>([]);
	let mics = $state<MicrophoneInfo[]>([]);

	onMount(() => {
		void api.listAudioApps().then((list) => (playing = list)).catch(() => (playing = []));
		void api.listMicrophones().then((list) => (mics = list)).catch(() => (mics = []));
	});

	const DEFAULT_DEVICE = '__default__';

	/** Discord first (the usual reason for split tracks), then whatever is playing sound now. */
	const suggestions = $derived.by(() => {
		const list = [{ exe: 'Discord.exe', name: 'Discord' }, ...playing.map((a) => ({ exe: a.exe, name: a.name }))];
		const seen = new Set<string>();
		return list.filter((s) => {
			const key = s.exe.toLowerCase();
			if (seen.has(key)) return false;
			seen.add(key);
			return true;
		});
	});
	const names = $derived(Object.fromEntries(suggestions.map((s) => [s.exe.toLowerCase(), s.name])));

	const micLabel = $derived.by(() => {
		if (draft.audio.mic.deviceId === null) {
			const d = mics.find((m) => m.isDefault);
			return d ? `Default device (${d.name})` : 'Default device';
		}
		return mics.find((m) => m.id === draft.audio.mic.deviceId)?.name ?? 'Unavailable device';
	});

	/** The track names a saved clip will carry with the current choices. */
	const trackNames = $derived.by(() => {
		const a = draft.audio;
		const out: string[] = [];
		if (a.mode === 'system') out.push('System');
		else {
			if (a.game) out.push('Game');
			for (const exe of a.apps) out.push(names[exe.toLowerCase()] ?? exe.replace(/\.exe$/i, ''));
			if (a.others) out.push('Other apps');
		}
		if (a.mic.enabled) out.push('Microphone');
		return out;
	});
</script>

<SettingsSection id="audio" title="Audio" description="Which sounds are recorded, and on which tracks.">
	<SettingsRow label="Tracks" labelId="audio-mode" description="Separate tracks let you turn Discord or music up, down or off in the editor.">
		<ToggleGroup.Root
			type="single"
			variant="outline"
			value={draft.audio.mode}
			onValueChange={(v) => v && (draft.audio.mode = v as AudioMode)}
			aria-labelledby="audio-mode"
		>
			<ToggleGroup.Item value="split" class="px-3">Separate tracks</ToggleGroup.Item>
			<ToggleGroup.Item value="system" class="px-3">One system track</ToggleGroup.Item>
		</ToggleGroup.Root>
		{#snippet below()}
			<p class="text-xs text-muted-foreground">
				Each clip will have {trackNames.length}
				{trackNames.length === 1 ? 'track' : 'tracks'}{trackNames.length > 0 ? ': ' : '.'}{trackNames.join(', ')}.
			</p>
		{/snippet}
	</SettingsRow>

	{#if draft.audio.mode === 'split'}
		<SettingsRow label="Game" controlId="audio-game" description="The game being recorded gets its own track.">
			<Switch id="audio-game" bind:checked={draft.audio.game} />
		</SettingsRow>
		<SettingsRow label="Other apps" controlId="audio-others" description="Everything not listed below, mixed into one track.">
			<Switch id="audio-others" bind:checked={draft.audio.others} />
		</SettingsRow>
		<SettingsRow advanced
			label="Apps with their own track"
			controlId="audio-app-input"
			description="Each app listed here is recorded on a separate track while it makes sound."
		>
			<ExeList
				bind:items={draft.audio.apps}
				inputId="audio-app-input"
				{suggestions}
				{names}
				suggestLabel="Add"
				emptyText="No apps have their own track. Their sound goes to Other apps."
			/>
		</SettingsRow>
	{/if}

	<SettingsRow label="Microphone" controlId="audio-mic" description="Recorded on its own track when on.">
		<div class="flex flex-col gap-2">
			<Switch id="audio-mic" bind:checked={draft.audio.mic.enabled} />
			<Select.Root
				type="single"
				value={draft.audio.mic.deviceId ?? DEFAULT_DEVICE}
				onValueChange={(v) => (draft.audio.mic.deviceId = v === DEFAULT_DEVICE ? null : v)}
				disabled={!draft.audio.mic.enabled}
			>
				<Select.Trigger class="w-full" aria-label="Microphone device">
					<span class="truncate">{micLabel}</span>
				</Select.Trigger>
				<Select.Content>
					<Select.Item value={DEFAULT_DEVICE} label="Default device">Default device</Select.Item>
					{#each mics as mic (mic.id)}
						<Select.Item value={mic.id} label={mic.name}>{mic.name}</Select.Item>
					{/each}
				</Select.Content>
			</Select.Root>
		</div>
	</SettingsRow>

	<SettingsRow advanced
		label="Microphone in the combined track"
		controlId="audio-mic-mix"
		description="Mix your voice into the combined first track too, so players that only play that track (Discord, browsers, most video players) have it. It keeps its own track either way."
	>
		<Switch id="audio-mic-mix" bind:checked={draft.audio.mic.inMix} disabled={!draft.audio.mic.enabled} />
	</SettingsRow>

	<SettingsRow label="Push to talk" controlId="audio-ptt" description={EXPLAIN.pushToTalk}>
		<Switch id="audio-ptt" bind:checked={draft.audio.mic.pushToTalk} disabled={!draft.audio.mic.enabled} />
	</SettingsRow>

	{#if draft.audio.mic.pushToTalk}
		<SettingsRow label="Push-to-talk key" labelId="audio-ptt-key" description="Hold it to be heard. A mouse button works too, for example a side button.">
			<HotkeyRecorder bind:value={draft.audio.mic.pushToTalkKey} labelledBy="audio-ptt-key" disabled={!draft.audio.mic.enabled} />
		</SettingsRow>
	{/if}

	<SettingsRow advanced label="Bitrate per track" controlId="audio-bitrate" description={EXPLAIN.audioBitrate}>
		<div class="flex flex-col gap-2">
			<div class="flex flex-wrap gap-1" role="group" aria-label="Audio bitrate presets">
				{#each AUDIO_BITRATE_PRESETS as kbps (kbps)}
					<Button
						variant="outline"
						size="xs"
						class="tnum"
						aria-pressed={draft.audio.bitrateKbps === kbps}
						onclick={() => (draft.audio.bitrateKbps = kbps)}
					>
						{kbps}
					</Button>
				{/each}
			</div>
			<NumberField id="audio-bitrate" label="Audio bitrate in kilobits per second" unit="kbit/s" min={8} step={16} bind:value={draft.audio.bitrateKbps} />
		</div>
	</SettingsRow>
</SettingsSection>
