<script lang="ts">
	import { untrack } from 'svelte';
	import { toast } from 'svelte-sonner';
	import { api, errorMessage } from '$lib/api';
	import type { VideoSettings } from '$lib/api/types';
	import * as Dialog from '$lib/components/ui/dialog';
	import { Button } from '$lib/components/ui/button';
	import { Spinner } from '$lib/components/ui/spinner';
	import { app, patchSettings } from '$lib/stores/app.svelte';
	import { CODEC_LABELS, formatBitrate } from '$lib/format/format';

	/**
	 * Shows the video settings a code holds next to the ones in use, and applies them on
	 * request. Unless the current settings are the defaults, they are first copied to the
	 * clipboard as a code, so going back is one paste away.
	 */
	let {
		incoming = $bindable(null),
		screenLabel,
		presetId = null
	}: {
		incoming: VideoSettings | null;
		screenLabel: string;
		/** The preset the code is for, or null for the default video settings. */
		presetId?: string | null;
	} = $props();

	const current = $derived(
		(presetId ? app.settings?.presets.find((p) => p.id === presetId)?.video : app.settings?.video) ?? null
	);
	let previousCode = $state<string | null>(null);
	let currentIsDefault = $state(true);
	let applying = $state(false);

	// Worked out on opening, so applying can write the clipboard straight from the click.
	// Only opening counts: applying changes the current settings and must not redo this.
	$effect(() => {
		if (!incoming) return;
		const live = untrack(() => current);
		if (!live) return;
		const now = $state.snapshot(live);
		previousCode = null;
		currentIsDefault = true;
		void (async () => {
			try {
				const defaults = await api.getDefaultSettings();
				currentIsDefault = same(now, defaults.video);
				if (!currentIsDefault) previousCode = await api.videoSettingsCode(now);
			} catch {
				currentIsDefault = false;
			}
		})();
	});

	function same(a: VideoSettings, b: VideoSettings): boolean {
		return (Object.keys(a) as (keyof VideoSettings)[]).every((k) => a[k] === b[k]);
	}

	type Row = { label: string; value: string };
	function rows(v: VideoSettings): { main: Row[]; fine: Row[] } {
		const rate =
			v.rateMode === 'cbr'
				? `CBR ${formatBitrate(v.bitrateKbps)}`
				: v.rateMode === 'vbr'
					? `VBR ${formatBitrate(v.bitrateKbps)}, peak ${formatBitrate(v.maxBitrateKbps)}`
					: v.rateMode === 'cq'
						? `Constant quality ${v.cq}, peak ${formatBitrate(v.maxBitrateKbps)}`
						: `Constant QP ${v.qp}`;
		return {
			main: [
				{ label: 'Codec', value: CODEC_LABELS[v.codec] },
				{ label: 'Resolution', value: v.width !== null && v.height !== null ? `${v.width}×${v.height}` : screenLabel },
				{ label: 'Other shapes', value: v.scaling === 'fit' ? 'Black bars' : 'Stretch to fill' },
				{ label: 'Frame rate', value: `${v.fps} fps` },
				{ label: 'Rate control', value: rate },
				{ label: 'Preset', value: `${v.preset} of 7` },
				{ label: 'Mouse cursor', value: v.captureCursor ? 'Shown' : 'Hidden' }
			],
			fine: [
				{ label: 'Tuning', value: { hq: 'High quality', ll: 'Low latency', ull: 'Ultra low latency' }[v.tune] },
				{ label: 'Multipass', value: { off: 'Off', quarter: 'Quarter resolution', full: 'Full resolution' }[v.multipass] },
				{ label: 'Look-ahead', value: v.lookahead ? `${v.lookahead} frames` : 'Off' },
				{ label: 'Spatial AQ', value: v.spatialAq ? `On, strength ${v.aqStrength}` : 'Off' },
				{ label: 'Temporal AQ', value: v.temporalAq ? 'On' : 'Off' },
				{ label: 'Keyframes', value: `Every ${v.keyframeSecs} s` }
			]
		};
	}

	const next = $derived(incoming ? rows(incoming) : null);
	const now = $derived(current ? rows(current) : null);
	const changes = $derived.by(() => {
		if (!next || !now) return 0;
		return [...next.main, ...next.fine].filter((r, i) => r.value !== [...now.main, ...now.fine][i].value).length;
	});

	async function apply() {
		if (!incoming || applying) return;
		applying = true;
		const video = $state.snapshot(incoming);
		const oldCode = previousCode;
		let copied = false;
		if (oldCode) {
			try {
				await navigator.clipboard.writeText(oldCode);
				copied = true;
			} catch {
				copied = false;
			}
		}
		const ok = await patchSettings((s) => {
			const preset = presetId ? s.presets.find((p) => p.id === presetId) : undefined;
			// A code leaves out what depends on this PC and screen; those stay as they are.
			const keep = (old: typeof video) => ({ ...structuredClone(video), hdr: old.hdr, skipStill: old.skipStill });
			if (preset) preset.video = keep(preset.video);
			else s.video = keep(s.video);
		});
		applying = false;
		if (!ok) return;
		incoming = null;
		if (oldCode && copied) {
			toast.success('Video settings applied', {
				description: `Your previous settings are in the clipboard as a code: ${oldCode}`,
				duration: 8000
			});
		} else if (oldCode) {
			toast.success('Video settings applied', {
				description: `The clipboard could not be written. Your previous settings were ${oldCode}`,
				duration: 12000
			});
		} else {
			toast.success('Video settings applied');
		}
	}

	function onOpenChange(open: boolean) {
		if (!open) incoming = null;
	}
</script>

{#snippet table(title: string, nextRows: Row[], nowRows: Row[])}
	<section aria-label={title} class="flex flex-col">
		<h3 class="mb-1 text-xs font-medium text-muted-foreground">{title}</h3>
		<div class="grid grid-cols-[7.5rem_minmax(0,1fr)_minmax(0,1fr)] text-sm" role="table">
			<div class="contents" role="row">
				<span class="py-1 text-xs text-subtle" role="columnheader">Setting</span>
				<span class="py-1 text-xs text-subtle" role="columnheader">From the code</span>
				<span class="py-1 text-xs text-subtle" role="columnheader">Yours now</span>
			</div>
			{#each nextRows as row, i (row.label)}
				{@const changed = row.value !== nowRows[i].value}
				<div class="contents" role="row">
					<span class="border-t border-line py-1.5 pr-3 text-muted-foreground" role="rowheader">{row.label}</span>
					<span class="tnum border-t border-line py-1.5 pr-3 {changed ? 'font-medium text-foreground' : 'text-subtle'}" role="cell">
						{#if changed}<span class="changed" aria-hidden="true"></span>{/if}{row.value}
						{#if changed}<span class="sr-only">(changes)</span>{/if}
					</span>
					<span class="tnum border-t border-line py-1.5 text-subtle {changed ? 'line-through decoration-line-strong' : ''}" role="cell">{nowRows[i].value}</span>
				</div>
			{/each}
		</div>
	</section>
{/snippet}

<Dialog.Root open={incoming !== null} {onOpenChange}>
	<Dialog.Content class="flex max-h-[88vh] flex-col sm:max-w-xl">
		<Dialog.Header>
			<Dialog.Title>Settings from a code</Dialog.Title>
			<Dialog.Description>
				{#if changes === 0}
					These are the same as your video settings.
				{:else}
					{changes === 1 ? 'One setting differs' : `${changes} settings differ`} from yours. Applying replaces your video settings
					and restarts the replay buffer.
				{/if}
			</Dialog.Description>
		</Dialog.Header>

		{#if next && now}
			<div class="-mx-4 flex min-h-0 flex-col gap-4 overflow-y-auto px-4">
				{@render table('Video', next.main, now.main)}
				{@render table('Encoder fine-tuning', next.fine, now.fine)}
			</div>
		{/if}

		<p class="text-xs text-muted-foreground">
			{#if currentIsDefault}
				Your current settings are the defaults, so nothing is copied.
			{:else}
				Your current settings are copied to the clipboard as a code first, so you can go back to them.
			{/if}
		</p>

		<Dialog.Footer>
			<Button variant="outline" onclick={() => (incoming = null)}>Cancel</Button>
			<Button onclick={apply} disabled={applying || changes === 0}>
				{#if applying}<Spinner data-icon="inline-start" />{/if}
				Apply these settings
			</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>

<style>
	.changed {
		display: inline-block;
		width: 6px;
		height: 6px;
		margin-right: 6px;
		border-radius: 999px;
		background: var(--text);
		vertical-align: 2px;
	}
</style>
