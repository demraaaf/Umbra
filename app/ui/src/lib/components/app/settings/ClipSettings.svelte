<script lang="ts">
	import type { Corner, Settings } from '$lib/api/types';
	import * as ToggleGroup from '$lib/components/ui/toggle-group';
	import { Button } from '$lib/components/ui/button';
	import { Slider } from '$lib/components/ui/slider';
	import { Switch } from '$lib/components/ui/switch';
	import { api, errorMessage } from '$lib/api';
	import { formatBytes } from '$lib/format/format';
	import { EXPLAIN } from '$lib/format/explain';
	import NumberField from '../fields/NumberField.svelte';
	import HotkeyRecorder from './HotkeyRecorder.svelte';
	import GameList from './GameList.svelte';
	import SettingsRow from './SettingsRow.svelte';
	import SettingsSection from './SettingsSection.svelte';

	let { draft }: { draft: Settings } = $props();

	const LENGTH_PRESETS = [15, 30, 60, 120, 300];

	/** Memory the replay buffer needs at the current quality, when the bitrate is known. */
	const bufferBytes = $derived.by(() => {
		const v = draft.video;
		if (v.rateMode === 'cqp') return null;
		const a = draft.audio;
		const tracks = (a.mode === 'system' ? 1 : (a.game ? 1 : 0) + a.apps.length + (a.others ? 1 : 0)) + (a.mic.enabled ? 1 : 0);
		const kbps = (v.rateMode === 'vbr' ? v.maxBitrateKbps : v.bitrateKbps) + tracks * a.bitrateKbps;
		return ((kbps * 1000) / 8) * (draft.clip.lengthSecs + draft.clip.postSecs);
	});

	const POST_PRESETS = [0, 2, 3, 5, 10];

	let picking = $state(false);
	let folderNote = $state('');
	async function chooseBufferFolder() {
		picking = true;
		try {
			const picked = await api.pickFolder();
			if (picked) draft.clip.bufferFolder = picked;
		} catch (error) {
			folderNote = `Could not open the picker: ${errorMessage(error)}`;
		} finally {
			picking = false;
		}
	}

	const volumePercent = $derived(Math.round(draft.clip.saveSoundVolume * 100));
</script>

<SettingsSection id="clip" title="Clip" description="What a hotkey press saves.">
	<SettingsRow label="Length" controlId="clip-length" description="{EXPLAIN.buffer} A saved clip is this long.">
		<div class="flex flex-col gap-2">
			<NumberField id="clip-length" label="Clip length in seconds" unit="s" min={1} max={3600} bind:value={draft.clip.lengthSecs} />
			<div class="flex flex-wrap gap-1">
				{#each LENGTH_PRESETS as secs (secs)}
					<Button
						variant="outline"
						aria-pressed={draft.clip.lengthSecs === secs}
						size="xs"
						class="tnum"
						onclick={() => (draft.clip.lengthSecs = secs)}
					>
						{secs < 60 ? `${secs} s` : `${secs / 60} min`}
					</Button>
				{/each}
			</div>
		</div>
		{#snippet below()}
			<p class="text-xs text-muted-foreground">
				{#if bufferBytes !== null}
					The buffer uses about <span class="tnum text-foreground">{formatBytes(bufferBytes)}</span>
					of {draft.clip.bufferOnDisk ? 'disk space' : 'memory'} at these settings.
				{:else}
					{draft.clip.bufferOnDisk ? 'Disk space' : 'Memory'} use depends on the scene with constant quality (CQP).
				{/if}
			</p>
		{/snippet}
	</SettingsRow>

	<SettingsRow
		label="After the hotkey"
		controlId="clip-post"
		description="Keeps recording this long after you press the hotkey, then saves. The moment right after a play ends up in the clip too."
	>
		<div class="flex flex-col gap-2">
			<NumberField id="clip-post" label="Seconds after the hotkey" unit="s" min={0} max={15} bind:value={draft.clip.postSecs} />
			<div class="flex flex-wrap gap-1">
				{#each POST_PRESETS as secs (secs)}
					<Button variant="outline" aria-pressed={draft.clip.postSecs === secs} size="xs" class="tnum" onclick={() => (draft.clip.postSecs = secs)}>
						{secs === 0 ? 'Off' : `${secs} s`}
					</Button>
				{/each}
			</div>
		</div>
		{#snippet below()}
			{#if draft.clip.postSecs > 0}
				<p class="tnum text-xs text-muted-foreground">
					A clip covers {draft.clip.lengthSecs} s before the hotkey and {draft.clip.postSecs} s after, {draft.clip.lengthSecs + draft.clip.postSecs} s in all.
				</p>
			{/if}
		{/snippet}
	</SettingsRow>

	<SettingsRow advanced
		label="Keep the buffer"
		labelId="clip-buffer-where"
		description="In memory is fastest. On disk frees the memory a long buffer at a high bitrate takes; it writes continuously, so an SSD with room to spare suits it best."
	>
		<div class="flex flex-col gap-2">
			<ToggleGroup.Root
				type="single"
				variant="outline"
				class="w-full"
				value={draft.clip.bufferOnDisk ? 'disk' : 'memory'}
				onValueChange={(v) => v && (draft.clip.bufferOnDisk = v === 'disk')}
				aria-labelledby="clip-buffer-where"
			>
				<ToggleGroup.Item value="memory" class="flex-1 px-2">In memory</ToggleGroup.Item>
				<ToggleGroup.Item value="disk" class="flex-1 px-2">On disk</ToggleGroup.Item>
			</ToggleGroup.Root>
			{#if draft.clip.bufferOnDisk}
				<div class="flex items-center gap-2">
					<span class="selectable min-w-0 flex-1 truncate text-xs text-muted-foreground" title={draft.clip.bufferFolder || "Umbra's cache folder"}>
						{draft.clip.bufferFolder || "Umbra's cache folder"}
					</span>
					{#if draft.clip.bufferFolder}
						<Button variant="ghost" size="xs" onclick={() => (draft.clip.bufferFolder = '')}>Default</Button>
					{/if}
					<Button variant="outline" size="sm" onclick={chooseBufferFolder} disabled={picking}>Choose</Button>
				</div>
				{#if folderNote}<p class="text-xs text-foreground" role="status">{folderNote}</p>{/if}
			{/if}
		</div>
	</SettingsRow>

	<SettingsRow label="Save hotkey" labelId="clip-hotkey" description="Works while a game is in fullscreen. Keyboard keys or mouse buttons.">
		<HotkeyRecorder bind:value={draft.clip.hotkey} defaultValue="Alt+F10" labelledBy="clip-hotkey" />
	</SettingsRow>

	<SettingsRow label="Save sound" controlId="clip-sound" description="A short sound confirms that a clip was saved.">
		<Switch id="clip-sound" bind:checked={draft.clip.saveSound} />
	</SettingsRow>

	<SettingsRow label="Sound volume" labelId="clip-volume">
		<div class="flex items-center gap-3">
			<Slider
				type="single"
				min={0}
				max={100}
				step={5}
				value={volumePercent}
				onValueChange={(v) => (draft.clip.saveSoundVolume = v / 100)}
				disabled={!draft.clip.saveSound}
				aria-labelledby="clip-volume"
			/>
			<span class="tnum w-10 shrink-0 text-right text-sm {draft.clip.saveSound ? 'text-foreground' : 'text-subtle'}">{volumePercent}%</span>
		</div>
	</SettingsRow>

	<SettingsRow
		label="Messages over the game"
		controlId="clip-overlay"
		description="A short note in a corner of the screen when a clip is saved or a recording starts or ends. It never takes the mouse or keyboard. Games in exclusive fullscreen draw over it; borderless and windowed games show it."
	>
		<Switch id="clip-overlay" bind:checked={draft.overlay.enabled} />
	</SettingsRow>

	{#if draft.overlay.enabled}
		<SettingsRow label="Where" labelId="clip-overlay-corner">
			<ToggleGroup.Root
				type="single"
				variant="outline"
				spacing={1}
				class="grid w-full grid-cols-2"
				value={draft.overlay.corner}
				onValueChange={(next) => next && (draft.overlay.corner = next as Corner)}
				aria-labelledby="clip-overlay-corner"
			>
				<ToggleGroup.Item value="topLeft">Top left</ToggleGroup.Item>
				<ToggleGroup.Item value="topRight">Top right</ToggleGroup.Item>
				<ToggleGroup.Item value="bottomLeft">Bottom left</ToggleGroup.Item>
				<ToggleGroup.Item value="bottomRight">Bottom right</ToggleGroup.Item>
			</ToggleGroup.Root>
		</SettingsRow>
	{/if}

	<SettingsRow
		label="Status over the game"
		labelId="clip-status"
		description="A small badge that stays on screen: REC and the time while recording, Replay and how much the buffer holds, or Off. It is kept out of your recordings and clips, and never takes the mouse or keyboard."
	>
		<ToggleGroup.Root
			type="single"
			variant="outline"
			spacing={1}
			class="grid w-full grid-cols-3"
			value={draft.overlay.status}
			onValueChange={(next) => next && (draft.overlay.status = next as 'off' | 'always' | 'games')}
			aria-labelledby="clip-status"
		>
			<ToggleGroup.Item value="off">Off</ToggleGroup.Item>
			<ToggleGroup.Item value="always">Always</ToggleGroup.Item>
			<ToggleGroup.Item value="games">In games</ToggleGroup.Item>
		</ToggleGroup.Root>
	</SettingsRow>

	{#if draft.overlay.status === 'games'}
		<SettingsRow
			label="Which games"
			controlId="clip-status-games"
			description="Show it only for these games. Pick one that is running or that you have clips of, or add a program. Leave the list empty to show it in any game Umbra detects."
		>
			<GameList
				inputId="clip-status-games"
				bind:items={draft.overlay.statusGames}
				paths
				folders
				emptyText="Any detected game."
			/>
		</SettingsRow>
	{/if}

	{#if draft.overlay.status !== 'off'}
		<SettingsRow label="Badge corner" labelId="clip-status-corner">
			<ToggleGroup.Root
				type="single"
				variant="outline"
				spacing={1}
				class="grid w-full grid-cols-2"
				value={draft.overlay.statusCorner}
				onValueChange={(next) => next && (draft.overlay.statusCorner = next as Corner)}
				aria-labelledby="clip-status-corner"
			>
				<ToggleGroup.Item value="topLeft">Top left</ToggleGroup.Item>
				<ToggleGroup.Item value="topRight">Top right</ToggleGroup.Item>
				<ToggleGroup.Item value="bottomLeft">Bottom left</ToggleGroup.Item>
				<ToggleGroup.Item value="bottomRight">Bottom right</ToggleGroup.Item>
			</ToggleGroup.Root>
		</SettingsRow>
	{/if}
</SettingsSection>
