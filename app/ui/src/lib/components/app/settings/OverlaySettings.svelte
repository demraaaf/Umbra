<script lang="ts">
	import { onMount } from 'svelte';
	import { api, errorMessage } from '$lib/api';
	import type { CameraInfo, OverlayShape, Settings } from '$lib/api/types';
	import { Button } from '$lib/components/ui/button';
	import * as Select from '$lib/components/ui/select';
	import { Switch } from '$lib/components/ui/switch';
	import * as ToggleGroup from '$lib/components/ui/toggle-group';
	import { DEFAULT_CAMERA_PLACEMENT, DEFAULT_LOGO_PLACEMENT, WHOLE } from '$lib/editor/overlay';
	import { DEFAULT_INPUT_OVERLAY, InputTimeline, compose, mockInputTrack } from '$lib/input/overlay';
	import { app } from '$lib/stores/app.svelte';
	import { toast } from 'svelte-sonner';
	import NumberField from '../fields/NumberField.svelte';
	import CameraStandIn from '../overlay/CameraStandIn.svelte';
	import CropBox from '../overlay/CropBox.svelte';
	import InputOverlayCanvas from '../overlay/InputOverlayCanvas.svelte';
	import LiveCamera from '../overlay/LiveCamera.svelte';
	import OverlayFrame from '../overlay/OverlayFrame.svelte';
	import SourcePreview from '../SourcePreview.svelte';
	import HotkeyRecorder from './HotkeyRecorder.svelte';
	import SettingsRow from './SettingsRow.svelte';
	import SettingsSection from './SettingsSection.svelte';

	let { draft }: { draft: Settings } = $props();

	const NONE = '__none__';

	let cameras = $state<CameraInfo[]>([]);
	let logoUrl = $state<string | null>(null);
	let logoAspect = $state(3);
	type Which = 'camera' | 'logo' | 'input';
	let selected = $state<Which>('camera');
	let previewWidth = $state(0);

	onMount(() => {
		void api.listCameras().then((list) => (cameras = list)).catch(() => (cameras = []));
	});

	/** The keyboard and mouse, playing made-up input so its look shows. */
	const sample = new InputTimeline(mockInputTrack(12));
	const inputComposition = $derived(draft.input.enabled ? compose(draft.input.overlay) : null);
	let clock = $state(0);
	$effect(() => {
		if (!inputComposition) return;
		let frame = 0;
		const started = performance.now();
		const tick = () => {
			clock = ((performance.now() - started) / 1000) % 12;
			frame = requestAnimationFrame(tick);
		};
		frame = requestAnimationFrame(tick);
		return () => cancelAnimationFrame(frame);
	});

	$effect(() => {
		const path = draft.logo.path;
		logoUrl = null;
		if (path) void api.fileUrl(path).then((url) => (logoUrl = url));
	});

	const camera = $derived(draft.camera);
	const chosen = $derived(cameras.find((c) => c.id === camera.deviceId) ?? null);
	const connected = $derived(chosen !== null);
	const cameraAspect = $derived(camera.width / camera.height);
	/** The recorded video's shape, as far as it is known: the running recording's, or 16:9. */
	const videoAspect = $derived(app.status && app.status.width > 0 ? app.status.width / app.status.height : 16 / 9);
	const previewHeight = $derived(previewWidth / videoAspect);

	// A software camera that films nothing now (OBS closed) says no sizes: the one set stays.
	const sizes = $derived(chosen?.sizes.length ? chosen.sizes : [{ width: camera.width, height: camera.height }]);
	const sizeKey = (s: { width: number; height: number }) => `${s.width}x${s.height}`;

	/** 720p where the camera has it, otherwise the closest size below, otherwise its smallest. */
	function startingSize(c: CameraInfo) {
		const fits = c.sizes.filter((s) => s.height <= 720);
		return c.sizes.find((s) => s.width === 1280 && s.height === 720) ?? fits[fits.length - 1] ?? c.sizes[0];
	}

	function chooseCamera(id: string) {
		if (id === NONE) {
			draft.camera.deviceId = null;
			return;
		}
		const c = cameras.find((x) => x.id === id);
		if (!c) return;
		draft.camera.deviceId = c.id;
		draft.camera.deviceName = c.name;
		const size = startingSize(c);
		if (size) {
			draft.camera.width = size.width;
			draft.camera.height = size.height;
		}
	}

	function chooseSize(key: string) {
		const size = sizes.find((s) => sizeKey(s) === key);
		if (size) {
			draft.camera.width = size.width;
			draft.camera.height = size.height;
		}
	}

	async function chooseLogo() {
		try {
			const path = await api.pickLogo();
			if (path) {
				draft.logo.path = path;
				selected = 'logo';
			}
		} catch (error) {
			toast.error('Could not use that image', { description: errorMessage(error) });
		}
	}

	const cameraLabel = $derived(
		camera.deviceId === null ? 'No camera' : (chosen?.name ?? `${camera.deviceName || 'Camera'} (not connected)`)
	);
	const borderPercent = $derived(Math.round(camera.placement.border * 1000) / 10);

	const shown = $derived<Record<Which, boolean>>({
		camera: camera.deviceId !== null,
		logo: !!draft.logo.path,
		input: inputComposition !== null
	});
	/** The one the reset button puts back: the selected one, if it is shown. */
	const resetting = $derived<Which>(
		shown[selected] ? selected : ((['camera', 'logo', 'input'] as const).find((w) => shown[w]) ?? 'camera')
	);
	const NAMES: Record<Which, string> = { camera: 'camera', logo: 'logo', input: 'keyboard and mouse' };

	/** Back to the default place and size; the look and the crop stay. */
	function resetPosition() {
		const [target, defaults] =
			resetting === 'camera'
				? [draft.camera, DEFAULT_CAMERA_PLACEMENT]
				: resetting === 'logo'
					? [draft.logo, DEFAULT_LOGO_PLACEMENT]
					: [draft.input.overlay, DEFAULT_INPUT_OVERLAY.placement];
		target.placement = { ...target.placement, x: defaults.x, y: defaults.y, width: defaults.width };
	}
</script>

<SettingsSection
	id="overlay"
	title="Overlays"
	description="Your camera, a logo and the keys you press, shown over your clips. None of them is burned into the recording: the editor can still move, resize or hide each one, and they are drawn in when you export or copy a clip."
>
	{#if shown.camera || shown.logo || shown.input}
		<SettingsRow
			label="Position"
			labelId="overlay-position"
			description="Where new clips show them. Drag to move, drag a corner to resize; edges and the middle pull them in, Alt drags freely."
		>
			<div class="flex flex-wrap gap-2">
				<Button variant="ghost" size="sm" onclick={resetPosition}>
					Reset {NAMES[resetting]} position
				</Button>
			</div>
			{#snippet below()}
				<div
					class="relative w-full overflow-hidden rounded-sm border border-line bg-black"
					style:height="{previewHeight}px"
					bind:clientWidth={previewWidth}
					role="group"
					aria-labelledby="overlay-position"
				>
					<SourcePreview target={draft.capture} maxWidth={820} fit="cover" empty="No preview of the screen" class="absolute inset-0 size-full opacity-80" />
					{#if previewWidth > 0}
						{#if draft.logo.path}
							<OverlayFrame
								bind:placement={draft.logo.placement}
								sourceAspect={logoAspect}
								frameWidth={previewWidth}
								frameHeight={previewHeight}
								editable
								label="Logo"
								selected={selected === 'logo'}
								onselect={() => (selected = 'logo')}
							>
								{#if logoUrl}
									<img
										src={logoUrl}
										alt=""
										class="block size-full"
										draggable="false"
										onload={(e) => {
											const img = e.currentTarget as HTMLImageElement;
											if (img.naturalWidth > 0) logoAspect = img.naturalWidth / img.naturalHeight;
										}}
									/>
								{/if}
							</OverlayFrame>
						{/if}
						{#if inputComposition}
							<OverlayFrame
								bind:placement={draft.input.overlay.placement}
								sourceAspect={inputComposition.width / inputComposition.height}
								frameWidth={previewWidth}
								frameHeight={previewHeight}
								editable
								label="Keyboard and mouse"
								selected={selected === 'input'}
								onselect={() => (selected = 'input')}
							>
								<InputOverlayCanvas timeline={sample} composition={inputComposition} prefs={draft.input.overlay} time={clock} />
							</OverlayFrame>
						{/if}
						{#if camera.deviceId !== null}
							<OverlayFrame
								bind:placement={draft.camera.placement}
								sourceAspect={cameraAspect}
								frameWidth={previewWidth}
								frameHeight={previewHeight}
								editable
								label="Camera"
								selected={selected === 'camera'}
								onselect={() => (selected = 'camera')}
							>
								{#if connected && camera.enabled}
									<LiveCamera name={chosen?.name ?? ''} />
								{:else}
									<CameraStandIn />
								{/if}
							</OverlayFrame>
						{/if}
					{/if}
				</div>
			{/snippet}
		</SettingsRow>
	{/if}
</SettingsSection>

<SettingsSection id="camera" page="overlay" title="Camera">
	<SettingsRow label="Camera" labelId="camera-device" description="Recorded as a video of its own next to the screen.">
		<Select.Root type="single" value={camera.deviceId ?? NONE} onValueChange={chooseCamera}>
			<Select.Trigger class="w-full" aria-labelledby="camera-device">
				<span class="truncate">{cameraLabel}</span>
			</Select.Trigger>
			<Select.Content>
				<Select.Item value={NONE} label="No camera">No camera</Select.Item>
				{#each cameras as c (c.id)}
					<Select.Item value={c.id} label={c.name}>
						{c.name}
						{#if c.api === 'DirectShow'}<span class="ml-auto pl-2 text-xs text-subtle" title="A software camera, read through DirectShow">DirectShow</span>{/if}
					</Select.Item>
				{/each}
				{#if camera.deviceId !== null && !connected}
					<Select.Item value={camera.deviceId} label={cameraLabel} disabled>{cameraLabel}</Select.Item>
				{/if}
			</Select.Content>
		</Select.Root>
		{#snippet below()}
			{#if cameras.length === 0}
				<p class="text-xs text-muted-foreground">No camera is connected. Plug one in and come back to this page.</p>
			{/if}
		{/snippet}
	</SettingsRow>

	{#if camera.deviceId !== null}
		<SettingsRow
			label="Record the camera"
			controlId="camera-on"
			description="Off turns the camera and its light off; recording carries on without it. The hotkey and the tray switch it too."
		>
			<Switch id="camera-on" bind:checked={draft.camera.enabled} />
		</SettingsRow>

		<SettingsRow label="Camera hotkey" labelId="camera-hotkey" description="Switches the camera on or off.">
			<HotkeyRecorder bind:value={draft.camera.hotkey} labelledBy="camera-hotkey" />
		</SettingsRow>

		<SettingsRow advanced
			label="Resolution"
			labelId="camera-size"
			description="Sizes the camera films at. 720p stays sharp at a usual overlay size; pick more if you make the camera big in the editor."
		>
			<Select.Root type="single" value={sizeKey(camera)} onValueChange={chooseSize}>
				<Select.Trigger class="tnum w-full" aria-labelledby="camera-size">
					<span>{camera.width}×{camera.height}</span>
				</Select.Trigger>
				<Select.Content>
					{#each sizes as s (sizeKey(s))}
						<Select.Item value={sizeKey(s)} label="{s.width}×{s.height}" class="tnum">{s.width}×{s.height}</Select.Item>
					{/each}
				</Select.Content>
			</Select.Root>
		</SettingsRow>

		<SettingsRow label="Crop" labelId="camera-crop" description="The part of the camera's picture shown. The editor can change it for each clip.">
			<div class="flex flex-col gap-2">
				<CropBox bind:rect={draft.camera.placement.crop} aspect={cameraAspect} label="Camera crop">
					{#if connected && camera.enabled}
						<LiveCamera name={chosen?.name ?? ''} />
					{:else}
						<CameraStandIn />
					{/if}
				</CropBox>
				<div class="flex justify-end">
					<Button variant="ghost" size="xs" onclick={() => (draft.camera.placement.crop = { ...WHOLE })}>Show all</Button>
				</div>
			</div>
		</SettingsRow>

		<SettingsRow label="Shape" labelId="camera-shape">
			<ToggleGroup.Root
				type="single"
				variant="outline"
				value={camera.placement.shape}
				onValueChange={(v) => v && (draft.camera.placement.shape = v as OverlayShape)}
				aria-labelledby="camera-shape"
			>
				<ToggleGroup.Item value="rectangle" class="px-3">Square corners</ToggleGroup.Item>
				<ToggleGroup.Item value="rounded" class="px-3">Rounded</ToggleGroup.Item>
				<ToggleGroup.Item value="circle" class="px-3">Circle</ToggleGroup.Item>
			</ToggleGroup.Root>
		</SettingsRow>

		<SettingsRow label="Border" controlId="camera-border" description="Thickness as a share of the camera's width; 0 for none.">
			<div class="flex items-center gap-2">
				<NumberField
					id="camera-border"
					label="Border thickness in percent of the camera's width"
					unit="%"
					min={0}
					max={10}
					value={borderPercent}
					oncommit={(v) => (draft.camera.placement.border = v / 100)}
					class="w-28"
				/>
				<input
					type="color"
					class="color"
					bind:value={draft.camera.placement.borderColor}
					aria-label="Border colour"
					disabled={camera.placement.border === 0}
				/>
			</div>
		</SettingsRow>

		<SettingsRow label="Mirror" controlId="camera-mirror" description="Flips the camera left to right, the way you see yourself in a mirror.">
			<Switch id="camera-mirror" bind:checked={draft.camera.placement.mirror} />
		</SettingsRow>
	{/if}
</SettingsSection>

<SettingsSection id="logo" page="overlay" title="Logo">
	<SettingsRow label="Image" labelId="logo-image" description="A picture over every new clip, such as a channel logo. PNG with transparency works best.">
		<div class="flex items-center gap-2">
			<Button variant="outline" size="sm" onclick={chooseLogo}>{draft.logo.path ? 'Choose another' : 'Choose image'}</Button>
			{#if draft.logo.path}
				<Button variant="ghost" size="sm" onclick={() => (draft.logo.path = null)}>Remove</Button>
			{/if}
		</div>
	</SettingsRow>
</SettingsSection>

<style>
	.color {
		width: 32px;
		height: 32px;
		padding: 2px;
		border: 1px solid var(--line-control);
		border-radius: var(--radius);
		background: var(--surface-raised);
		cursor: pointer;
	}
	.color:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}
</style>
