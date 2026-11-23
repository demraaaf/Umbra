<script lang="ts">
	import { onMount } from 'svelte';
	import { toast } from 'svelte-sonner';
	import CopyIcon from '@lucide/svelte/icons/copy';
	import CheckIcon from '@lucide/svelte/icons/check';
	import GaugeIcon from '@lucide/svelte/icons/gauge';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import { api, errorMessage } from '$lib/api';
	import type {
		MonitorInfo,
		Multipass,
		RateMode,
		Scaling,
		Settings,
		Tune,
		VideoCodec,
		VideoSettings
	} from '$lib/api/types';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { Slider } from '$lib/components/ui/slider';
	import { Switch } from '$lib/components/ui/switch';
	import * as ToggleGroup from '$lib/components/ui/toggle-group';
	import { app, applyDraft, draftIsDirty } from '$lib/stores/app.svelte';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import ExeList from './ExeList.svelte';
	import { CODEC_LABELS, formatBitrate } from '$lib/format/format';
	import { CODEC_EXPLAIN, EXPLAIN, RATE_MODE_EXPLAIN, VIDEO_BITRATE_PRESETS } from '$lib/format/explain';
	import NumberField from '../fields/NumberField.svelte';
	import BenchmarkDialog from './BenchmarkDialog.svelte';
	import LoadTestDialog from './LoadTestDialog.svelte';
	import ActivityIcon from '@lucide/svelte/icons/activity';
	import CodePreviewDialog from './CodePreviewDialog.svelte';
	import SettingsRow from './SettingsRow.svelte';
	import SettingsSection from './SettingsSection.svelte';
	import { showAdvanced } from '$lib/stores/settingsNav.svelte';

	let { draft }: { draft: Settings } = $props();

	// ---------- Presets ----------
	// The page edits one set of video settings at a time: the default ones (the desktop and
	// any game no preset lists) or a preset's.

	let selected = $state('default');
	const preset = $derived(selected === 'default' ? null : (draft.presets.find((p) => p.id === selected) ?? null));
	const v = $derived(preset ? preset.video : draft.video);

	function addPreset() {
		const id = `preset-${Date.now().toString(36)}`;
		draft.presets.push({ id, name: 'New preset', games: [], video: structuredClone($state.snapshot(draft.video)) });
		selected = id;
	}

	function removePreset() {
		draft.presets = draft.presets.filter((p) => p.id !== selected);
		selected = 'default';
	}

	const CODECS: VideoCodec[] = ['h264', 'hevc', 'av1'];
	const FPS_PRESETS = [60, 120, 144, 240, 360];
	const SIZE_PRESETS = [
		{ label: '720p', width: 1280, height: 720 },
		{ label: '1080p', width: 1920, height: 1080 },
		{ label: '1440p', width: 2560, height: 1440 },
		{ label: '4K', width: 3840, height: 2160 }
	];
	const STRETCHED_PRESETS = [
		{ label: '1440×1080', width: 1440, height: 1080 },
		{ label: '1280×960', width: 1280, height: 960 },
		{ label: '1280×1024', width: 1280, height: 1024 }
	];
	const RATE_LABELS: Record<RateMode, string> = { cbr: 'CBR', vbr: 'VBR', cq: 'CQ', cqp: 'CQP' };
	const TUNE_LABELS: Record<Tune, string> = { hq: 'High quality', ll: 'Low latency', ull: 'Ultra low' };
	const MULTIPASS_LABELS: Record<Multipass, string> = { off: 'Off', quarter: 'Quarter', full: 'Full' };

	const supported = $derived(app.systemInfo?.codecs ?? CODECS);
	const custom = $derived(v.width !== null && v.height !== null);
	const windowCapture = $derived(draft.capture.kind === 'window');

	let monitors = $state<MonitorInfo[]>([]);
	let setupOpen = $state(false);
	let loadOpen = $state(false);

	onMount(() => {
		api.listMonitors().then((list) => (monitors = list)).catch(() => {});
	});

	/** The monitor being recorded, for "Same as screen". */
	const screen = $derived.by(() => {
		const device = draft.capture.kind === 'monitor' ? draft.capture.device : '';
		return monitors.find((m) => device && m.device === device) ?? monitors.find((m) => m.primary) ?? monitors[0];
	});
	const stretchedNow = $derived(
		!!screen && !windowCapture && (screen.width !== screen.nativeWidth || screen.height !== screen.nativeHeight)
	);

	function setResolutionMode(mode: string) {
		if (mode === 'source') {
			v.width = null;
			v.height = null;
		} else if (mode === 'custom' && !custom) {
			v.width = screen?.nativeWidth || app.status?.width || 1920;
			v.height = screen?.nativeHeight || app.status?.height || 1080;
		}
	}

	function setBitrate(kbps: number) {
		v.bitrateKbps = kbps;
		if (v.maxBitrateKbps < kbps) v.maxBitrateKbps = Math.round(kbps * 1.5);
	}

	// ---------- Simple mode ----------
	// Without advanced settings the page offers three quality levels instead of codecs and
	// bitrates. The levels use the setup's own formula, so "Balanced" fits any size and rate.

	const advanced = $derived(showAdvanced());
	const SIMPLE_FPS = [60, 120, 144, 240];
	const SIMPLE_SIZES = SIZE_PRESETS.slice(0, 3);
	const LEVELS = [
		{ value: 'small', label: 'Smaller files', factor: 0.8 },
		{ value: 'balanced', label: 'Balanced', factor: 1 },
		{ value: 'best', label: 'Best', factor: 1.4 }
	];
	const outSize = $derived(
		custom ? [v.width!, v.height!] : [screen?.nativeWidth || app.status?.width || 1920, screen?.nativeHeight || app.status?.height || 1080]
	);

	/** Mirrors `bitrate_kbps` in benchmark.rs for "a bit of everything". */
	function levelKbps(factor: number): number {
		const codec = v.codec === 'h264' ? 1.35 : v.codec === 'av1' ? 0.85 : 1;
		const pixels = ((outSize[0] * outSize[1]) / (1920 * 1080)) ** 0.8;
		return 22_500 * factor * codec * pixels * (v.fps / 60) ** 0.6;
	}
	const roundKbps = (kbps: number) => Math.max(2, Math.round(kbps / 1000)) * 1000;

	/** The level closest to the bitrate now; none for the constant-quality modes. */
	const level = $derived(
		v.rateMode === 'cbr' || v.rateMode === 'vbr'
			? LEVELS.reduce((a, b) =>
					Math.abs(Math.log(v.bitrateKbps / levelKbps(a.factor))) <= Math.abs(Math.log(v.bitrateKbps / levelKbps(b.factor))) ? a : b
				).value
			: ''
	);

	function setLevel(value: string) {
		const kbps = roundKbps(levelKbps(LEVELS.find((l) => l.value === value)!.factor));
		if (v.rateMode !== 'cbr') v.rateMode = 'vbr';
		v.bitrateKbps = kbps;
		v.maxBitrateKbps = roundKbps(kbps * 1.5);
	}

	/** Makes a change to the size, rate or codec and scales the bitrate with it, so the quality stays. */
	function keepQuality(change: () => void) {
		const before = levelKbps(1);
		change();
		if (v.rateMode !== 'cbr' && v.rateMode !== 'vbr') return;
		const ratio = levelKbps(1) / before;
		v.bitrateKbps = roundKbps(v.bitrateKbps * ratio);
		v.maxBitrateKbps = Math.max(v.bitrateKbps, roundKbps(v.maxBitrateKbps * ratio));
	}

	const sizeKey = $derived(custom ? `${v.width}x${v.height}` : 'source');
	const smallerCodec = $derived(supported.includes('hevc') ? 'hevc' : supported.includes('av1') ? 'av1' : null);

	// ---------- Share code ----------

	let code = $state('');
	let copied = $state(false);
	let incoming = $state('');
	let incomingError = $state<string | null>(null);
	let loading = $state(false);
	/** Settings read from a code, shown for confirmation. */
	let preview = $state<VideoSettings | null>(null);

	// The code follows the draft, so it always describes what is on screen.
	$effect(() => {
		const video = $state.snapshot(v);
		const timer = setTimeout(() => {
			api.videoSettingsCode(video).then((c) => (code = c)).catch(() => (code = ''));
		}, 120);
		return () => clearTimeout(timer);
	});

	async function copyCode() {
		try {
			await navigator.clipboard.writeText(code);
			copied = true;
			setTimeout(() => (copied = false), 1600);
		} catch (error) {
			toast.error('Could not copy the code', { description: errorMessage(error) });
		}
	}

	async function loadCode(event: SubmitEvent) {
		event.preventDefault();
		if (!incoming.trim() || loading) return;
		loading = true;
		incomingError = null;
		try {
			preview = await api.readVideoSettingsCode(incoming);
			incoming = '';
		} catch (error) {
			incomingError = errorMessage(error);
		} finally {
			loading = false;
		}
	}
</script>

<SettingsSection
	id="video"
	title="Video"
	description="Presets give kinds of games their own settings: smooth clips for shooters, detailed ones for story games."
>
	<div class="mb-3 flex flex-col gap-3">
		<div class="flex flex-wrap items-center gap-1" role="radiogroup" aria-label="Video settings for">
			<button type="button" class="preset-tab" role="radio" aria-checked={selected === 'default'} onclick={() => (selected = 'default')}>
				Everything else
			</button>
			{#each draft.presets as p (p.id)}
				<button type="button" class="preset-tab" role="radio" aria-checked={selected === p.id} onclick={() => (selected = p.id)}>
					{p.name || 'Unnamed preset'}
					<span class="tnum text-subtle">{p.games.length}</span>
				</button>
			{/each}
			<Button variant="ghost" size="sm" onclick={addPreset}>
				<PlusIcon data-icon="inline-start" />
				New preset
			</Button>
		</div>
		{#if preset}
			<div class="flex flex-col gap-3 rounded-md border border-line bg-panel p-3">
				<div class="flex items-center gap-2">
					<Input bind:value={preset.name} aria-label="Preset name" class="max-w-72" />
					<Button variant="ghost" size="sm" class="ml-auto text-destructive" onclick={removePreset}>
						<Trash2Icon data-icon="inline-start" />
						Delete preset
					</Button>
				</div>
				<div class="flex flex-col gap-1.5">
					<span id="preset-games" class="text-xs text-muted-foreground">
						Games that use it. A game found here counts as a game even when it is not fullscreen.
					</span>
					<ExeList inputId="preset-games-input" bind:items={preset.games} paths emptyText="No games yet. Add the program name, such as cs2.exe, or choose the program." />
				</div>
			</div>
		{:else}
			<p class="text-xs text-muted-foreground">
				Used for the desktop and every game that is not in a preset. The settings below apply to the choice above.
			</p>
		{/if}
	</div>

	<div class="setup mb-3 flex flex-col gap-3 rounded-md border border-line bg-panel p-3 @lg:flex-row @lg:items-center">
		<GaugeIcon class="size-5 shrink-0 text-muted-foreground" aria-hidden="true" />
		<div class="min-w-0 flex-1">
			<p class="text-sm font-medium text-foreground">Find the best settings for this PC</p>
			<p class="mt-0.5 text-xs text-muted-foreground">
				Answer four questions, and Umbra measures your graphics card's encoder to pick the best-looking settings that
				don't drop frames. About 10 seconds.
			</p>
			<p class="tnum mt-1 text-xs text-foreground">
				Now: {custom ? `${v.width}×${v.height}` : screen ? `${screen.nativeWidth}×${screen.nativeHeight}` : 'screen size'} · {v.fps} fps ·
				{CODEC_LABELS[v.codec]}{v.rateMode === 'cbr' || v.rateMode === 'vbr' ? ` · ${formatBitrate(v.bitrateKbps)}` : ''}
			</p>
		</div>
		<Button variant="outline" class="shrink-0 self-start @lg:self-center" onclick={() => (setupOpen = true)}>Run the setup</Button>
	</div>

	{#if advanced}
	<div class="mb-3 flex flex-col gap-3 rounded-md border border-line bg-panel p-3 @lg:flex-row @lg:items-center">
		<ActivityIcon class="size-5 shrink-0 text-muted-foreground" aria-hidden="true" />
		<div class="min-w-0 flex-1">
			<p class="text-sm font-medium text-foreground">Test under a game's load</p>
			<p class="mt-0.5 text-xs text-muted-foreground">
				A stand-in game loads the graphics card, processor and memory while Umbra records with the settings below. Shows
				what recording costs the game, whether frames drop, and what to change. About 25 seconds.
			</p>
		</div>
		<Button variant="outline" class="shrink-0 self-start @lg:self-center" onclick={() => (loadOpen = true)} disabled={draftIsDirty()} title={draftIsDirty() ? 'Apply your changes first; the test uses the applied settings' : ''}>
			Start the test
		</Button>
	</div>
	{/if}

	{#if !advanced}
	<SettingsRow label="Quality" labelId="video-level" description="Better quality makes bigger clips. The estimate above shows how big.">
		<ToggleGroup.Root class="w-full" type="single" variant="outline" value={level} onValueChange={(next) => next && setLevel(next)} aria-labelledby="video-level">
			{#each LEVELS as l (l.value)}
				<ToggleGroup.Item value={l.value} class="flex-1 px-2">{l.label}</ToggleGroup.Item>
			{/each}
		</ToggleGroup.Root>
		{#snippet below()}
			{#if !level}
				<p class="text-xs text-muted-foreground">Set to a constant quality under advanced settings. Picking a level here replaces it.</p>
			{/if}
		{/snippet}
	</SettingsRow>

	<SettingsRow label="Frame rate" labelId="video-simple-fps" description="Smoother clips cost more of the graphics card.">
		<ToggleGroup.Root
			class="w-full"
			type="single"
			variant="outline"
			value={SIMPLE_FPS.includes(v.fps) ? String(v.fps) : ''}
			onValueChange={(next) => next && keepQuality(() => (v.fps = Number(next)))}
			aria-labelledby="video-simple-fps"
		>
			{#each SIMPLE_FPS as fps (fps)}
				<ToggleGroup.Item value={String(fps)} class="tnum flex-1 px-1">{fps} fps</ToggleGroup.Item>
			{/each}
		</ToggleGroup.Root>
		{#snippet below()}
			{#if !SIMPLE_FPS.includes(v.fps)}
				<p class="tnum text-xs text-muted-foreground">Now {v.fps} fps, set under advanced settings.</p>
			{:else if v.fps >= 240}
				<p class="text-xs text-foreground">Needs a high-end PC. This can cause dropped frames in recordings and in-game.</p>
			{/if}
		{/snippet}
	</SettingsRow>

	<SettingsRow label="Resolution" labelId="video-simple-size" description="Smaller sizes are lighter and make smaller files.">
		<ToggleGroup.Root
			class="w-full"
			type="single"
			variant="outline"
			value={sizeKey === 'source' || SIMPLE_SIZES.some((p) => `${p.width}x${p.height}` === sizeKey) ? sizeKey : ''}
			onValueChange={(next) =>
				next &&
				keepQuality(() => {
					const p = SIMPLE_SIZES.find((p) => `${p.width}x${p.height}` === next);
					v.width = p?.width ?? null;
					v.height = p?.height ?? null;
				})}
			aria-labelledby="video-simple-size"
		>
			<ToggleGroup.Item value="source" class="flex-1 px-1">{windowCapture ? 'Window' : 'Screen'}</ToggleGroup.Item>
			{#each SIMPLE_SIZES as p (p.label)}
				<ToggleGroup.Item value="{p.width}x{p.height}" class="tnum flex-1 px-1">{p.label}</ToggleGroup.Item>
			{/each}
		</ToggleGroup.Root>
		{#snippet below()}
			<p class="tnum text-xs text-subtle">
				{#if custom}{v.width}×{v.height}{:else if screen && !windowCapture}Your screen: {screen.nativeWidth}×{screen.nativeHeight}{:else}The window's own size{/if}
			</p>
		{/snippet}
	</SettingsRow>

	<SettingsRow label="Plays on" labelId="video-simple-codec" description="Discord previews and some browsers play only the first. The second looks as good at about half the size.">
		<ToggleGroup.Root
			class="w-full"
			type="single"
			variant="outline"
			value={v.codec === 'h264' ? 'everywhere' : 'smaller'}
			onValueChange={(next) => next && keepQuality(() => (v.codec = next === 'everywhere' ? 'h264' : (smallerCodec ?? v.codec)))}
			aria-labelledby="video-simple-codec"
		>
			<ToggleGroup.Item value="everywhere" class="flex-1 px-2">Everywhere</ToggleGroup.Item>
			<ToggleGroup.Item value="smaller" class="flex-1 px-2" disabled={!smallerCodec} title={smallerCodec ? '' : 'This graphics card cannot encode HEVC or AV1'}>
				Smaller files
			</ToggleGroup.Item>
		</ToggleGroup.Root>
	</SettingsRow>
	{:else}

	<SettingsRow label="Codec" labelId="video-codec" description={EXPLAIN.codec}>
		<ToggleGroup.Root
			type="single"
			variant="outline"
			value={v.codec}
			onValueChange={(next) => next && (v.codec = next as VideoCodec)}
			aria-labelledby="video-codec"
		>
			{#each CODECS as codec (codec)}
				<ToggleGroup.Item value={codec} class="min-w-16" disabled={!supported.includes(codec)} title={supported.includes(codec) ? '' : 'Not supported by this GPU'}>
					{CODEC_LABELS[codec]}
				</ToggleGroup.Item>
			{/each}
		</ToggleGroup.Root>
		{#snippet below()}
			<dl class="grid grid-cols-[48px_1fr] gap-x-3 gap-y-1 text-xs">
				{#each CODECS as codec (codec)}
					<dt class="{v.codec === codec ? 'text-foreground' : 'text-subtle'} font-medium">{CODEC_LABELS[codec]}</dt>
					<dd class={v.codec === codec ? 'text-foreground' : 'text-muted-foreground'}>
						{CODEC_EXPLAIN[codec]}{supported.includes(codec) ? '' : ' This graphics card cannot encode it.'}
					</dd>
				{/each}
			</dl>
		{/snippet}
	</SettingsRow>

	<SettingsRow
		label="Resolution"
		labelId="video-resolution"
		description={windowCapture
			? 'Same as the window keeps the window’s size; a custom size is scaled on the GPU before encoding.'
			: 'Same as the screen records at your monitor’s own resolution, even while a game runs it at a lower or stretched one. A custom size is scaled on the GPU.'}
	>
		<div class="flex flex-col gap-2">
			<ToggleGroup.Root
			class="w-full"
				type="single"
				variant="outline"
				value={custom ? 'custom' : 'source'}
				onValueChange={(v) => v && setResolutionMode(v)}
				aria-labelledby="video-resolution"
			>
				<ToggleGroup.Item value="source" class="min-w-20 flex-1">{windowCapture ? 'Same as the window' : 'Same as the screen'}</ToggleGroup.Item>
				<ToggleGroup.Item value="custom" class="min-w-20 flex-1">Custom</ToggleGroup.Item>
			</ToggleGroup.Root>
			{#if custom && v.width !== null && v.height !== null}
				<div class="flex items-center gap-2">
					<NumberField label="Width" unit="px" min={16} max={16384} even bind:value={v.width} />
					<span class="text-muted-foreground" aria-hidden="true">&times;</span>
					<NumberField label="Height" unit="px" min={16} max={16384} even bind:value={v.height} />
				</div>
				<div class="flex flex-wrap gap-1" role="group" aria-label="Common sizes">
					{#each [...SIZE_PRESETS, ...STRETCHED_PRESETS] as size (size.label)}
						<Button
							variant="outline"
							size="xs"
							class="tnum"
							aria-pressed={v.width === size.width && v.height === size.height}
							onclick={() => {
								v.width = size.width;
								v.height = size.height;
							}}
						>
							{size.label}
						</Button>
					{/each}
				</div>
			{:else if screen && !windowCapture}
				<p class="tnum text-xs text-subtle">
					{screen.nativeWidth}×{screen.nativeHeight}{stretchedNow ? `, while the screen runs at ${screen.width}×${screen.height} right now` : ''}
				</p>
			{/if}
		</div>
	</SettingsRow>

	<SettingsRow label="Other shapes" labelId="video-scaling" description={EXPLAIN.scaling}>
		<ToggleGroup.Root
			class="w-full"
			type="single"
			variant="outline"
			value={v.scaling}
			onValueChange={(next) => next && (v.scaling = next as Scaling)}
			aria-labelledby="video-scaling"
		>
			<ToggleGroup.Item value="stretch" class="min-w-20 flex-1">Stretch to fill</ToggleGroup.Item>
			<ToggleGroup.Item value="fit" class="min-w-20 flex-1">Black bars</ToggleGroup.Item>
		</ToggleGroup.Root>
		{#snippet below()}
			<div class="flex items-center gap-3 text-xs text-muted-foreground" aria-hidden="true">
				<span class="shape" class:on={v.scaling === 'stretch'}><span class="pic stretch"></span></span>
				<span class="shape" class:on={v.scaling === 'fit'}><span class="pic fit"></span></span>
				<span>A 4:3 game in a 16:9 clip</span>
			</div>
		{/snippet}
	</SettingsRow>

	<SettingsRow
		label="HDR screens"
		labelId="video-hdr"
		description="With Windows' HDR on, games are captured in HDR. Converted to SDR, the video looks right on every screen and site; kept as HDR10 it keeps the highlights for HDR screens and YouTube HDR, and needs HEVC or AV1."
	>
		<ToggleGroup.Root
			class="w-full"
			type="single"
			variant="outline"
			value={v.hdr}
			onValueChange={(next) => next && (v.hdr = next as 'sdr' | 'hdr10')}
			aria-labelledby="video-hdr"
		>
			<ToggleGroup.Item value="sdr" class="min-w-20 flex-1">Convert to SDR</ToggleGroup.Item>
			<ToggleGroup.Item value="hdr10" class="min-w-20 flex-1">Keep HDR10</ToggleGroup.Item>
		</ToggleGroup.Root>
		{#snippet below()}
			{#if v.hdr === 'hdr10' && v.codec === 'h264'}
				<p class="text-xs text-foreground">H.264 cannot carry HDR, so it is converted to SDR. Pick HEVC or AV1 to keep it.</p>
			{/if}
		{/snippet}
	</SettingsRow>

	<SettingsRow label="Frame rate" controlId="video-fps" description="Any whole number from 1 to 360.">
		<div class="flex flex-col gap-2">
			<NumberField id="video-fps" label="Frames per second" unit="fps" min={1} max={360} bind:value={v.fps} />
			<ToggleGroup.Root
				type="single"
				variant="outline"
				size="sm"
				value={FPS_PRESETS.includes(v.fps) ? String(v.fps) : ''}
				onValueChange={(next) => next && (v.fps = Number(next))}
				aria-label="Frame rate presets"
				class="w-full"
			>
				{#each FPS_PRESETS as fps (fps)}
					<ToggleGroup.Item value={String(fps)} class="tnum flex-1 px-1">{fps}</ToggleGroup.Item>
				{/each}
			</ToggleGroup.Root>
		</div>
		{#snippet below()}
			{#if v.fps >= 240}
				<p class="flex items-start gap-2 text-xs text-foreground" role="note">
					<TriangleAlertIcon class="mt-px size-4 shrink-0" aria-hidden="true" />
					<span>Needs a high-end PC. This can cause dropped frames in recordings and in-game.</span>
				</p>
			{/if}
		{/snippet}
	</SettingsRow>

	<SettingsRow label="Skip still frames" controlId="video-skip-still" description={EXPLAIN.skipStill}>
		<Switch id="video-skip-still" bind:checked={v.skipStill} />
	</SettingsRow>

	<SettingsRow label="Rate control" labelId="video-rate" description={EXPLAIN.rateControl}>
		<ToggleGroup.Root
			class="w-full"
			type="single"
			variant="outline"
			value={v.rateMode}
			onValueChange={(next) => next && (v.rateMode = next as RateMode)}
			aria-labelledby="video-rate"
		>
			{#each ['cbr', 'vbr', 'cq', 'cqp'] as mode (mode)}
				<ToggleGroup.Item value={mode} class="min-w-14 flex-1">{RATE_LABELS[mode as RateMode]}</ToggleGroup.Item>
			{/each}
		</ToggleGroup.Root>
		{#snippet below()}
			<p class="text-xs text-muted-foreground">{RATE_MODE_EXPLAIN[v.rateMode]}</p>
		{/snippet}
	</SettingsRow>

	{#if v.rateMode === 'cqp'}
		<SettingsRow label="Quantiser (QP)" labelId="video-qp" description="{EXPLAIN.qp} About 10 is near-lossless; 18 to 23 is a common range.">
			<div class="flex items-center gap-3">
				<Slider type="single" min={0} max={51} step={1} bind:value={v.qp} aria-labelledby="video-qp" />
				<NumberField label="QP" min={0} max={51} bind:value={v.qp} class="w-20 shrink-0" />
			</div>
		</SettingsRow>
	{:else if v.rateMode === 'cq'}
		<SettingsRow label="Quality level" labelId="video-cq" description="Lower is better and larger. 18 to 25 suits most games; below 15 files grow quickly.">
			<div class="flex flex-col gap-2">
				<div class="flex items-center gap-3">
					<Slider type="single" min={1} max={51} step={1} bind:value={v.cq} aria-labelledby="video-cq" />
					<NumberField label="Quality level" min={1} max={51} bind:value={v.cq} class="w-20 shrink-0" />
				</div>
				<div class="flex justify-between text-xs text-subtle">
					<span>Best</span>
					<span>Smallest</span>
				</div>
			</div>
		</SettingsRow>
	{:else}
		<SettingsRow
			label={v.rateMode === 'vbr' ? 'Target bitrate' : 'Bitrate'}
			controlId="video-bitrate"
			description={EXPLAIN.bitrate}
		>
			<div class="flex flex-col gap-2">
				<div class="grid grid-cols-3 gap-1" role="group" aria-label="Bitrate presets">
					{#each VIDEO_BITRATE_PRESETS as preset (preset.kbps)}
						<Button
							variant="outline"
							size="sm"
							class="h-auto flex-col items-start gap-0 px-2 py-1 text-left"
							aria-pressed={v.bitrateKbps === preset.kbps}
							onclick={() => setBitrate(preset.kbps)}
						>
							<span class="tnum text-xs font-medium">{preset.kbps / 1000} Mbit/s</span>
							<span class="text-xs font-normal text-muted-foreground">{preset.label}</span>
						</Button>
					{/each}
				</div>
				<NumberField
					id="video-bitrate"
					label="Bitrate in kilobits per second"
					unit="kbit/s"
					min={1}
					step={1000}
					bind:value={v.bitrateKbps}
					oncommit={(kbps) => {
						if (v.maxBitrateKbps < kbps) v.maxBitrateKbps = kbps;
					}}
				/>
				<p class="tnum text-xs text-subtle">{formatBitrate(v.bitrateKbps)}, custom values welcome.</p>
			</div>
		</SettingsRow>
	{/if}
	{#if v.rateMode === 'vbr' || v.rateMode === 'cq'}
		<SettingsRow
			label="Peak bitrate"
			controlId="video-peak"
			description="The most the encoder may use in a busy moment; it also sets how much memory the replay buffer reserves."
		>
			<div class="flex flex-col gap-1">
				<NumberField
					id="video-peak"
					label="Peak bitrate in kilobits per second"
					unit="kbit/s"
					min={v.rateMode === 'vbr' ? v.bitrateKbps : 1000}
					step={1000}
					bind:value={v.maxBitrateKbps}
				/>
				<p class="tnum text-xs text-subtle">{formatBitrate(v.maxBitrateKbps)}</p>
			</div>
		</SettingsRow>
	{/if}

	<SettingsRow label="Encoder preset" labelId="video-preset" description={EXPLAIN.preset}>
		<div class="flex flex-col gap-2">
			<div class="flex items-center gap-3">
				<Slider type="single" min={1} max={7} step={1} bind:value={v.preset} aria-labelledby="video-preset" />
				<span class="tnum w-4 shrink-0 text-right text-sm text-foreground">{v.preset}</span>
			</div>
			<div class="flex justify-between text-xs text-subtle">
				<span>Fastest</span>
				<span>Best quality</span>
			</div>
		</div>
	</SettingsRow>

			<SettingsRow label="Tuning" labelId="video-tune" description={EXPLAIN.tune}>
				<ToggleGroup.Root
			class="w-full"
					type="single"
					variant="outline"
					value={v.tune}
					onValueChange={(next) => next && (v.tune = next as Tune)}
					aria-labelledby="video-tune"
				>
					{#each ['hq', 'll', 'ull'] as tune (tune)}
						<ToggleGroup.Item value={tune} class="flex-1 px-2">{TUNE_LABELS[tune as Tune]}</ToggleGroup.Item>
					{/each}
				</ToggleGroup.Root>
			</SettingsRow>

			<SettingsRow label="Multipass" labelId="video-multipass" description={EXPLAIN.multipass}>
				<ToggleGroup.Root
			class="w-full"
					type="single"
					variant="outline"
					value={v.multipass}
					onValueChange={(next) => next && (v.multipass = next as Multipass)}
					aria-labelledby="video-multipass"
				>
					{#each ['off', 'quarter', 'full'] as pass (pass)}
						<ToggleGroup.Item value={pass} class="flex-1 px-2">{MULTIPASS_LABELS[pass as Multipass]}</ToggleGroup.Item>
					{/each}
				</ToggleGroup.Root>
			</SettingsRow>

			<SettingsRow label="Look-ahead" controlId="video-lookahead" description={EXPLAIN.lookahead}>
				<NumberField id="video-lookahead" label="Look-ahead in frames" unit="frames" min={0} max={32} bind:value={v.lookahead} />
			</SettingsRow>

			<SettingsRow label="Spatial AQ" controlId="video-saq" description={EXPLAIN.spatialAq}>
				<Switch id="video-saq" bind:checked={v.spatialAq} />
			</SettingsRow>

			{#if v.spatialAq}
				<SettingsRow label="AQ strength" labelId="video-aq-strength" description={EXPLAIN.aqStrength}>
					<div class="flex items-center gap-3">
						<Slider type="single" min={1} max={15} step={1} bind:value={v.aqStrength} aria-labelledby="video-aq-strength" />
						<span class="tnum w-5 shrink-0 text-right text-sm text-foreground">{v.aqStrength}</span>
					</div>
				</SettingsRow>
			{/if}

			<SettingsRow label="Temporal AQ" controlId="video-taq" description={EXPLAIN.temporalAq}>
				<Switch id="video-taq" bind:checked={v.temporalAq} />
			</SettingsRow>

			<SettingsRow label="Keyframe interval" controlId="video-keyframes" description={EXPLAIN.keyframes}>
				<NumberField id="video-keyframes" label="Seconds between keyframes" unit="s" min={1} max={10} bind:value={v.keyframeSecs} />
			</SettingsRow>

	<SettingsRow
		label="Settings code"
		labelId="video-code-label"
		description="Every video setting on this page as one code. Send it to a friend, or keep it to come back to."
	>
		<Button variant="outline" class="w-full" onclick={copyCode} disabled={!code}>
			{#if copied}<CheckIcon data-icon="inline-start" />Copied{:else}<CopyIcon data-icon="inline-start" />Copy the code{/if}
		</Button>
		{#snippet below()}
			<Input
				id="video-code"
				value={code}
				readonly
				aria-labelledby="video-code-label"
				class="tnum font-mono text-xs tracking-wide"
				onfocus={(e) => e.currentTarget.select()}
			/>
		{/snippet}
	</SettingsRow>

	<SettingsRow label="Use a code" controlId="video-code-in" description="Paste a code to see its video settings next to yours, then choose whether to use them.">
		<form class="flex flex-col gap-1.5" onsubmit={loadCode}>
			<div class="flex items-center gap-2">
				<Input
					id="video-code-in"
					bind:value={incoming}
					placeholder="UMBRA-…"
					autocomplete="off"
					spellcheck="false"
					class="font-mono text-xs"
					aria-invalid={incomingError ? 'true' : undefined}
					aria-describedby={incomingError ? 'video-code-error' : undefined}
					oninput={() => (incomingError = null)}
				/>
				<Button type="submit" variant="outline" disabled={!incoming.trim() || loading}>Preview</Button>
			</div>
			{#if incomingError}
				<p id="video-code-error" class="text-xs text-destructive" role="alert">{incomingError}</p>
			{/if}
		</form>
	</SettingsRow>
	{/if}
</SettingsSection>

<LoadTestDialog
	bind:open={loadOpen}
	presetId={preset?.id ?? null}
	presetName={preset ? preset.name : 'default'}
	onapply={(next) => {
		if (preset) preset.video = next;
		else draft.video = next;
	}}
/>
<BenchmarkDialog
	bind:open={setupOpen}
	video={v}
	{windowCapture}
	{screen}
	onapply={async (next) => {
		if (preset) preset.video = next;
		else draft.video = next;
		await applyDraft();
	}}
/>
<CodePreviewDialog
	bind:incoming={preview}
	presetId={preset?.id ?? null}
	screenLabel={windowCapture ? 'Same as the window' : screen ? `Same as the screen (${screen.nativeWidth}×${screen.nativeHeight})` : 'Same as the screen'}
/>

<style>
	.preset-tab {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		height: 28px;
		padding: 0 10px;
		border-radius: 6px;
		border: 1px solid var(--line);
		font-size: 13px;
		color: var(--text-muted);
		transition:
			background-color var(--dur-fast) var(--ease-out),
			border-color var(--dur-fast) var(--ease-out),
			color var(--dur-fast) var(--ease-out);
	}
	.preset-tab:hover {
		background: var(--surface-raised);
		color: var(--text);
	}
	.preset-tab[aria-checked='true'] {
		background: var(--surface-on);
		border-color: var(--line-control);
		color: var(--text);
	}
	.preset-tab:focus-visible {
		outline: 2px solid var(--focus);
		outline-offset: 2px;
	}
	/* Two tiny 16:9 frames showing what each choice does to a 4:3 picture. */
	.shape {
		display: block;
		width: 32px;
		height: 18px;
		border: 1px solid var(--line-strong);
		border-radius: 2px;
		background: #000;
		overflow: hidden;
		opacity: 0.55;
		transition: opacity var(--dur-fast) var(--ease-out);
	}
	.shape.on {
		opacity: 1;
		border-color: var(--text-subtle);
	}
	.pic {
		display: block;
		height: 100%;
		background: linear-gradient(160deg, #5c5c5c, #2a2a2a);
	}
	.pic.stretch {
		width: 100%;
	}
	.pic.fit {
		width: 75%;
		margin: 0 auto;
	}
</style>
