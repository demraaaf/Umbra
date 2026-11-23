<script lang="ts">
	import type { ScreenshotFormat, Settings } from '$lib/api/types';
	import { Switch } from '$lib/components/ui/switch';
	import * as ToggleGroup from '$lib/components/ui/toggle-group';
	import { EXPLAIN } from '$lib/format/explain';
	import HotkeyRecorder from './HotkeyRecorder.svelte';
	import SettingsRow from './SettingsRow.svelte';
	import SettingsSection from './SettingsSection.svelte';

	let { draft }: { draft: Settings } = $props();
</script>

<SettingsSection id="screenshots" title="Screenshots" description="A still of what is being captured, saved next to your clips.">
	<SettingsRow label="Screenshot hotkey" labelId="shot-hotkey" description="Works while a game is in fullscreen. Keyboard keys or mouse buttons.">
		<HotkeyRecorder bind:value={draft.screenshot.hotkey} defaultValue="Alt+F11" labelledBy="shot-hotkey" />
	</SettingsRow>

	<SettingsRow label="Format" labelId="shot-format" description={EXPLAIN.screenshotFormat}>
		<ToggleGroup.Root
			type="single"
			variant="outline"
			value={draft.screenshot.format}
			onValueChange={(v) => v && (draft.screenshot.format = v as ScreenshotFormat)}
			aria-labelledby="shot-format"
		>
			<ToggleGroup.Item value="png" class="min-w-16">PNG</ToggleGroup.Item>
			<ToggleGroup.Item value="jpg" class="min-w-16">JPG</ToggleGroup.Item>
		</ToggleGroup.Root>
	</SettingsRow>

	<SettingsRow label="Play the save sound" controlId="shot-sound" description="Uses the same sound and volume as saving a clip.">
		<Switch id="shot-sound" bind:checked={draft.screenshot.sound} />
	</SettingsRow>
</SettingsSection>
