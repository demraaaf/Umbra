<script lang="ts">
	import { Button } from '$lib/components/ui/button';
	import { Switch } from '$lib/components/ui/switch';
	import type { EditSession } from '$lib/editor/session.svelte';
	import { WHOLE } from '$lib/editor/overlay';
	import { formatTimecode } from '$lib/format/format';
	import CameraStandIn from '../overlay/CameraStandIn.svelte';
	import CropBox from '../overlay/CropBox.svelte';
	import SyncedCamera from '../overlay/SyncedCamera.svelte';

	/** The clip's camera and logo: on or off, the camera's crop, and where it is hidden. */
	let { session }: { session: EditSession } = $props();

	const placement = $derived(session.cameraPlacement);
	const selection = $derived(session.selection);
	const offset = $derived(session.cameraVideo?.stretches[0]?.[0] ?? 0);
	const hiddenCount = $derived(session.cameraHidden.length);
</script>

{#if placement || session.logo || session.input}
	<section class="flex flex-col gap-2 border-b border-line p-3" aria-labelledby="sec-camera">
		<h3 id="sec-camera" class="text-xs font-medium text-muted-foreground">Camera, logo and input</h3>

		{#if placement}
			<div class="flex items-center justify-between gap-2">
				<span id="camera-shown" class="text-sm text-foreground">Camera</span>
				<Switch
					checked={!session.cameraOff}
					onCheckedChange={(v) => session.setCameraOff(!v)}
					disabled={session.busy}
					aria-labelledby="camera-shown"
				/>
			</div>
			{#if !session.cameraOff}
				<p class="text-xs text-subtle">Drag it on the picture to move it; drag a corner to resize it.</p>
				<span id="camera-crop" class="text-xs text-muted-foreground">Part of the camera shown</span>
				<CropBox
					bind:rect={() => placement.crop, (crop) => session.setCameraPlacement({ ...placement, crop })}
					aspect={session.cameraAspect}
					label="Camera crop"
				>
					{#if session.cameraVideo}
						<SyncedCamera url={session.cameraVideo.url} {offset} playback={session.playback} />
					{:else}
						<CameraStandIn />
					{/if}
				</CropBox>
				<div class="flex flex-wrap gap-1">
					<Button
						variant="outline"
						size="xs"
						disabled={!selection || session.busy}
						title={selection ? '' : 'Pick a stretch on the timeline first: Shift and drag'}
						onclick={() => selection && session.hideCamera(selection.start, selection.end)}
					>
						Hide in selection
					</Button>
					<Button variant="ghost" size="xs" disabled={session.busy} onclick={() => session.setCameraPlacement({ ...placement, crop: { ...WHOLE } })}>
						Show all of it
					</Button>
					<Button variant="ghost" size="xs" disabled={session.busy} onclick={() => session.resetCameraPlacement()}>Reset</Button>
				</div>
				{#if hiddenCount > 0}
					<ul class="flex flex-col gap-1" aria-label="Where the camera is hidden">
						{#each session.cameraHidden as range (range.start)}
							<li class="flex items-center justify-between gap-2 text-xs">
								<span class="tnum text-muted-foreground">Hidden {formatTimecode(range.start)} to {formatTimecode(range.end)}</span>
								<Button variant="ghost" size="xs" disabled={session.busy} onclick={() => session.showCameraAt(range.start)}>Show</Button>
							</li>
						{/each}
					</ul>
				{/if}
			{:else}
				<p class="text-xs text-subtle">Left out of the export.</p>
			{/if}
		{/if}

		{#if session.logo}
			<div class="flex items-center justify-between gap-2">
				<span id="logo-shown" class="text-sm text-foreground">Logo</span>
				<Switch
					checked={!session.logoOff}
					onCheckedChange={(v) => session.setLogoOff(!v)}
					disabled={session.busy}
					aria-labelledby="logo-shown"
				/>
			</div>
		{/if}

		{#if session.input}
			<div class="flex items-center justify-between gap-2">
				<span id="input-shown" class="text-sm text-foreground">Keyboard and mouse</span>
				<Switch
					checked={!session.inputOff}
					onCheckedChange={(v) => session.setInputOff(!v)}
					disabled={session.busy}
					aria-labelledby="input-shown"
				/>
			</div>
			{#if session.inputOff}
				<p class="text-xs text-subtle">Left out of the export.</p>
			{/if}
		{/if}
	</section>
{/if}
