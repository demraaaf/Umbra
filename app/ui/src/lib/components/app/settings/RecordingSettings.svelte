<script lang="ts">
	import type { RecordingMode, SessionSource, Settings } from '$lib/api/types';
	import * as ToggleGroup from '$lib/components/ui/toggle-group';
	import { Switch } from '$lib/components/ui/switch';
	import { formatBytes } from '$lib/format/format';
	import HotkeyRecorder from './HotkeyRecorder.svelte';
	import ChoiceList, { type Choice } from './ChoiceList.svelte';
	import ExeList from './ExeList.svelte';
	import SettingsRow from './SettingsRow.svelte';
	import SettingsSection from './SettingsSection.svelte';

	let { draft }: { draft: Settings } = $props();

	const MODES: Choice[] = [
		{ value: 'always', label: 'Always', detail: 'Keep the buffer running whenever Umbra is open.' },
		{ value: 'games', label: 'Only while a game is running', detail: 'Start and stop by itself with your games.' },
		{ value: 'manual', label: 'Manual', detail: 'Keep the buffer off until you turn it on in the panel. Record still works.' }
	];

	/** Disk space an hour of recording takes at the current video and audio settings. */
	const perHour = $derived.by(() => {
		const v = draft.video;
		if (v.rateMode === 'cqp') return null;
		// Constant quality usually stays well under its peak.
		const videoKbps = v.rateMode === 'cq' ? v.maxBitrateKbps * 0.6 : v.bitrateKbps;
		const a = draft.audio;
		const tracks = a.mode === 'system' ? 1 : (a.game ? 1 : 0) + a.apps.length + (a.others ? 1 : 0) + 1;
		const audioKbps = (tracks + (a.mic.enabled ? 1 : 0)) * a.bitrateKbps;
		return ((videoKbps + audioKbps) * 1000 * 3600) / 8;
	});
</script>

<SettingsSection id="recording" title="Recording" description="When the replay buffer runs, and recordings to a file.">
	<SettingsRow label="Replay buffer" labelId="recording-mode">
		<ChoiceList
			name="recording-mode"
			labelledBy="recording-mode"
			choices={MODES}
			bind:value={() => draft.recording.mode, (v) => v && (draft.recording.mode = v as RecordingMode)}
		/>
	</SettingsRow>

	<SettingsRow
		label="Record hotkey"
		labelId="recording-record-hotkey"
		description="Starts or stops a recording of what is being captured: the screen or window chosen under Capture. Clips and screenshots keep working while it records."
	>
		<HotkeyRecorder bind:value={draft.recording.recordHotkey} defaultValue="Alt+F9" labelledBy="recording-record-hotkey" />
	</SettingsRow>

	<SettingsRow advanced
		label="Pause hotkey"
		labelId="recording-pause-hotkey"
		description="Pauses the running recording or session, and resumes it. What happens meanwhile is left out; the file carries on without a gap."
	>
		<HotkeyRecorder bind:value={draft.recording.pauseHotkey} defaultValue="Alt+F8" labelledBy="recording-pause-hotkey" />
	</SettingsRow>

	<SettingsRow
		label="Full game sessions"
		controlId="recording-sessions"
		description="Record each game from the moment it is found until it closes. The save hotkey still saves clips and also marks the moment in the recording, to find it again later."
	>
		<Switch id="recording-sessions" bind:checked={draft.recording.fullSessions} />
	</SettingsRow>

	<SettingsRow advanced
		label="Sessions record"
		labelId="recording-session-source"
		description="With the game's window only, clips can keep the whole screen (set under Capture) while the session holds just the game. It also keeps recording the game while you are in another window."
	>
		<ToggleGroup.Root
			type="single"
			variant="outline"
			class="w-full"
			value={draft.recording.sessionSource}
			onValueChange={(next) => next && (draft.recording.sessionSource = next as SessionSource)}
			aria-labelledby="recording-session-source"
		>
			<ToggleGroup.Item value="capture" class="flex-1 px-2">Same as clips</ToggleGroup.Item>
			<ToggleGroup.Item value="gameWindow" class="flex-1 px-2">The game's window</ToggleGroup.Item>
		</ToggleGroup.Root>
	</SettingsRow>

	<SettingsRow
		label="Session hotkey"
		labelId="recording-session-hotkey"
		description="Starts recording the game you are playing until it closes, or stops it. Without a game it records like the record hotkey."
	>
		<HotkeyRecorder bind:value={draft.recording.sessionHotkey} defaultValue="Alt+F12" labelledBy="recording-session-hotkey" />
		{#snippet below()}
			{#if perHour}
				<p class="tnum text-xs text-muted-foreground">
					At the current video and audio settings, an hour of recording takes about {formatBytes(perHour)} of disk space.
				</p>
			{/if}
		{/snippet}
	</SettingsRow>

	<SettingsRow label="Launch at startup" controlId="recording-startup" description="Start Umbra when you sign in to Windows.">
		<Switch id="recording-startup" bind:checked={draft.recording.launchAtStartup} />
	</SettingsRow>

	<SettingsRow label="Start minimized" controlId="recording-minimized" description="Open in the tray instead of showing this window.">
		<Switch id="recording-minimized" bind:checked={draft.recording.startMinimized} />
	</SettingsRow>

	<SettingsRow advanced
		label="Games"
		controlId="recording-games"
		description="Programs to treat as games, on top of automatic detection. Used by the mode that records only while a game is running."
	>
		<ExeList
			inputId="recording-games"
			bind:items={draft.recording.games}
			paths
			emptyText="No extra games. Running games are also detected automatically."
		/>
	</SettingsRow>

	<SettingsRow advanced
		label="Never record"
		controlId="recording-never"
		description="Programs that never count as games, even when they fill the screen: no session records them, and the game-only replay buffer ignores them. Add a program name, a program's path, or a folder for everything in it."
	>
		<ExeList
			inputId="recording-never"
			bind:items={draft.recording.neverRecord}
			paths
			folders
			emptyText="Nothing excluded. Video players and other full-screen programs can go here."
		/>
	</SettingsRow>
</SettingsSection>
