<script lang="ts">
	import type { Settings } from '$lib/api/types';
	import { Button } from '$lib/components/ui/button';
	import * as ToggleGroup from '$lib/components/ui/toggle-group';
	import { Switch } from '$lib/components/ui/switch';
	import NumberField from '$lib/components/app/fields/NumberField.svelte';
	import SettingsRow from './SettingsRow.svelte';
	import SettingsSection from './SettingsSection.svelte';

	let { draft }: { draft: Settings } = $props();

	const PRESETS = [1, 2, 5, 10, 15, 30];
	/** Logical processors, as the webview reports them. */
	const cores = typeof navigator !== 'undefined' ? navigator.hardwareConcurrency || 8 : 8;
</script>

<SettingsSection id="player" title="Player and editor" description="Watching clips in the player, moving through them in the editor, and exporting.">
	<SettingsRow
		label="Arrow keys skip"
		controlId="player-seek"
		description="How far ← and → jump in the player and the editor. Shift and an arrow key steps one frame in the player, and selects in the editor."
	>
		<div class="flex flex-col gap-2">
			<NumberField id="player-seek" label="Arrow key skip in seconds" unit="s" min={1} max={600} bind:value={draft.player.seekSecs} />
			<div class="flex flex-wrap gap-1">
				{#each PRESETS as secs (secs)}
					<Button
						variant="outline"
						aria-pressed={draft.player.seekSecs === secs}
						size="xs"
						class="tnum"
						onclick={() => (draft.player.seekSecs = secs)}
					>
						{secs} s
					</Button>
				{/each}
			</div>
		</div>
	</SettingsRow>

	<SettingsRow advanced
		label="Export threads"
		labelId="player-threads"
		description="Processor threads that decode the video while exporting; the graphics card encodes it. Auto uses one per core and is usually fastest. Fewer threads leave more of the processor to a game you are playing."
	>
		<div class="flex flex-col gap-2">
			<ToggleGroup.Root
				type="single"
				variant="outline"
				class="w-full"
				value={draft.export.threads === 0 ? 'auto' : 'custom'}
				onValueChange={(next) => next && (draft.export.threads = next === 'auto' ? 0 : Math.max(1, Math.round(cores / 2)))}
				aria-labelledby="player-threads"
			>
				<ToggleGroup.Item value="auto" class="flex-1">Auto ({cores})</ToggleGroup.Item>
				<ToggleGroup.Item value="custom" class="flex-1">Set a number</ToggleGroup.Item>
			</ToggleGroup.Root>
			{#if draft.export.threads > 0}
				<NumberField label="Export threads" unit="threads" min={1} max={256} bind:value={draft.export.threads} />
			{/if}
		</div>
	</SettingsRow>

	<SettingsRow advanced
		label="Burn in replaces the original"
		controlId="player-burn-replace"
		description="Burn in (right-click a clip in the library) draws its camera, keyboard and mouse or logo into the picture. Normally that makes a new file next to the clip, named “(burned)”. With this on, the new file takes the clip’s place once it checks out, and the original goes to the Recycle Bin (unless Storage says to skip it)."
	>
		<Switch id="player-burn-replace" bind:checked={draft.export.burnReplaceOriginal} />
	</SettingsRow>
</SettingsSection>
