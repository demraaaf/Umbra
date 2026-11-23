<script lang="ts">
	import { Switch } from '$lib/components/ui/switch';
	import { Label } from '$lib/components/ui/label';
	import { Button } from '$lib/components/ui/button';
	import { app, openSettingsAt, patchSettings } from '$lib/stores/app.svelte';
	import { bindingLabels } from '$lib/format/hotkey';
	import Keys from '../Keys.svelte';
	import PanelSection from './PanelSection.svelte';

	const camera = $derived(app.settings?.camera);
	const live = $derived(app.status?.cameraLive ?? false);
	const recording = $derived(app.status?.state === 'recording');
	const problem = $derived(app.status?.cameraProblem ?? null);

	const stateText = $derived.by(() => {
		if (!camera?.deviceId) return 'None';
		if (!camera.enabled) return 'Off';
		if (!recording) return 'Not recording';
		if (problem) return 'Not connected';
		return live ? 'Live' : 'Starting';
	});
</script>

<PanelSection id="panel-camera" title="Camera">
	{#snippet action()}
		<span class="flex items-center gap-1 text-xs whitespace-nowrap {live ? 'text-foreground' : 'text-subtle'}" role="status" aria-live="polite">
			<span class="dot" class:live aria-hidden="true"></span>
			{stateText}
		</span>
	{/snippet}
	{#if camera}
		{#if camera.deviceId}
			<div class="flex items-center justify-between gap-2">
				<Label for="panel-camera-on" class="min-w-0 truncate text-xs font-normal text-foreground" title={camera.deviceName}>
					{camera.deviceName || 'Record the camera'}
				</Label>
				<Switch
					id="panel-camera-on"
					size="sm"
					checked={camera.enabled}
					onCheckedChange={(v) => patchSettings((s) => (s.camera.enabled = v))}
				/>
			</div>
			{#if problem && camera.enabled && recording}
				<p class="text-xs text-danger" role="alert">{problem}</p>
			{/if}
			<div class="flex items-center justify-between gap-2 text-xs">
				<span class="text-muted-foreground">On or off</span>
				<Keys all={bindingLabels(camera.hotkey)} label="Camera hotkey" />
			</div>
		{:else}
			<p class="text-xs text-subtle">No camera is set up.</p>
			<Button variant="outline" size="sm" onclick={() => openSettingsAt('overlay')}>Set up a camera</Button>
		{/if}
	{/if}
</PanelSection>

<style>
	.dot {
		width: 8px;
		height: 8px;
		border-radius: 999px;
		border: 1px solid var(--text-subtle);
	}
	.dot.live {
		background: var(--text);
		border-color: var(--text);
	}
</style>
