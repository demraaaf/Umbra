<script lang="ts">
	import type { Settings } from '$lib/api/types';
	import Keys from '$lib/components/app/Keys.svelte';
	import ShortcutList from '$lib/components/app/ShortcutList.svelte';
	import { bindingLabels } from '$lib/format/hotkey';
	import { editorShortcuts, libraryShortcuts, playerShortcuts, windowShortcuts } from '$lib/format/shortcuts';
	import SettingsSection from './SettingsSection.svelte';

	/** Every control in one place. Hotkeys come from the saved settings, not the draft. */
	let { settings }: { settings: Settings } = $props();

	const seek = $derived(settings.player.seekSecs);
	const length = $derived(settings.clip.lengthSecs);
	const lengthText = $derived(length < 60 ? `${length} seconds` : `${Math.round((length / 60) * 10) / 10} minutes`);
</script>

<SettingsSection id="help" title="Help" description="How to get around Umbra with the mouse and the keyboard.">
	<div class="flex flex-col gap-6 border-t border-line pt-4">
		<section class="flex flex-col gap-2" aria-labelledby="help-recording">
			<h3 id="help-recording" class="text-sm font-medium text-foreground">Recording</h3>
			<p class="text-xs text-muted-foreground">
				While Umbra records, it keeps the last {lengthText} in memory and nothing on disk. Pressing the save hotkey writes
				that stretch as a clip. A recording writes everything to a file until you stop it, and a full session records a
				game from start to end; the save hotkey keeps working during both and also marks the moment in the recording. The
				hotkeys work while a game is in fullscreen.
			</p>
			<div class="grid grid-cols-[minmax(0,auto)_1fr] items-center gap-x-6 gap-y-3 text-xs">
				<span class="justify-self-start"><Keys all={bindingLabels(settings.clip.hotkey)} label="Save hotkey" /></span>
				<span class="text-muted-foreground">Save the last {lengthText} as a clip</span>
				<span class="justify-self-start"><Keys all={bindingLabels(settings.screenshot.hotkey)} label="Screenshot hotkey" /></span>
				<span class="text-muted-foreground">Take a screenshot of what is being captured</span>
				<span class="justify-self-start"><Keys all={bindingLabels(settings.recording.recordHotkey)} label="Record hotkey" /></span>
				<span class="text-muted-foreground">Start or stop recording what is being captured</span>
				<span class="justify-self-start"><Keys all={bindingLabels(settings.recording.sessionHotkey)} label="Session hotkey" /></span>
				<span class="text-muted-foreground">Start or stop recording the game session; it stops by itself when the game closes</span>
				<span class="justify-self-start"><Keys all={bindingLabels(settings.recording.pauseHotkey)} label="Pause hotkey" /></span>
				<span class="text-muted-foreground">Pause or resume the recording or session</span>
				{#if settings.audio.mic.enabled && settings.audio.mic.pushToTalk}
					<span class="justify-self-start"><Keys all={bindingLabels(settings.audio.mic.pushToTalkKey)} label="Push-to-talk key" /></span>
					<span class="text-muted-foreground">Hold to record the microphone (push-to-talk)</span>
				{/if}
			</div>
			<p class="text-xs text-subtle">
				Change them under Clip, Screenshots, Recording and Audio. The recording panel on the right switches what is captured.
				Bookmarks show on the timeline when a recording opens in the player or the editor.
			</p>
		</section>

		<section class="flex flex-col gap-2" aria-labelledby="help-library">
			<h3 id="help-library" class="text-sm font-medium text-foreground">Clips and albums</h3>
			<p class="text-xs text-muted-foreground">
				Select clips to delete them or add them to an album. An album can hide its clips from the Clips list, which keeps
				the list for what you have not sorted yet. In the screenshot viewer, ← and → go to the previous and next item.
			</p>
			<ShortcutList items={libraryShortcuts()} />
		</section>

		<section class="flex flex-col gap-2" aria-labelledby="help-player">
			<h3 id="help-player" class="text-sm font-medium text-foreground">Player</h3>
			<p class="text-xs text-muted-foreground">
				A click on a clip plays it. The arrow keys skip {seek} {seek === 1 ? 'second' : 'seconds'}; change that under Player.
			</p>
			<ShortcutList items={playerShortcuts(seek)} />
		</section>

		<section class="flex flex-col gap-2" aria-labelledby="help-editor">
			<h3 id="help-editor" class="text-sm font-medium text-foreground">Editor</h3>
			<p class="text-xs text-muted-foreground">
				Trim sets where the clip starts and ends. Cutting out removes a part from the middle and joins what is left. Each
				audio track has its own volume; the first track, Mix, is what most players play and is rebuilt from the others on
				export. With enough room, each track shows its waveform under the timeline.
			</p>
			<ShortcutList items={editorShortcuts(seek)} />
		</section>

		<section class="flex flex-col gap-2" aria-labelledby="help-window">
			<h3 id="help-window" class="text-sm font-medium text-foreground">Window</h3>
			<ShortcutList items={windowShortcuts()} />
		</section>

		<section class="flex flex-col gap-2" aria-labelledby="help-credits">
			<h3 id="help-credits" class="text-sm font-medium text-foreground">Credits</h3>
			<p class="text-xs text-muted-foreground">
				Umbra is made by <span class="text-foreground">Coooollat</span> and is free software under the GNU GPL 3.0. Its code
				is on GitHub at <span class="selectable text-foreground">github.com/Coooollat/Umbra</span>. Video and audio go through FFmpeg, used
				under the LGPL 2.1; its license is in the licenses folder next to Umbra.
			</p>
		</section>
	</div>
</SettingsSection>
