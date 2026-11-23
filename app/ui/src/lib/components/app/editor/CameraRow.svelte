<script lang="ts">
	import type { EditSession } from '$lib/editor/session.svelte';
	import { formatTimecode } from '$lib/format/format';

	/**
	 * The camera on the timeline's scale: a bar where it filmed, broken where it was off, and
	 * the stretches hidden in the editor marked; clicking one shows the camera there again.
	 */
	let {
		session,
		start,
		span,
		width
	}: { session: EditSession; start: number; span: number; width: number } = $props();

	const pxPerSec = $derived(width > 0 && span > 0 ? width / span : 0);
	const xOf = (t: number) => (t - start) * pxPerSec;

	function band(a: number, b: number): { left: number; width: number } | null {
		const left = Math.max(xOf(a), 0);
		const right = Math.min(xOf(b), width);
		return right > left ? { left, width: right - left } : null;
	}

	/** Where it filmed; the mock, without a camera file, filmed throughout. */
	const filmed = $derived(
		(session.cameraVideo?.stretches ?? (session.playback.simulated ? [[0, session.duration] as [number, number]] : []))
			.map(([a, b]) => band(a, b))
			.filter((b) => b !== null)
	);
	const hidden = $derived(
		session.cameraHidden.map((r) => ({ r, b: band(r.start, r.end) })).filter((x) => x.b !== null)
	);
	const dropped = $derived(session.cameraOff);
</script>

<div class="relative h-5 border-t border-line" class:dropped>
	{#each filmed as b, i (i)}
		<span class="filmed" style:left="{b!.left}px" style:width="{b!.width}px" aria-hidden="true"></span>
	{/each}
	{#each hidden as h (h.r.start)}
		<button
			type="button"
			class="hidden-range"
			style:left="{h.b!.left}px"
			style:width="{h.b!.width}px"
			title="Camera hidden from {formatTimecode(h.r.start)} to {formatTimecode(h.r.end)}. Click to show it again."
			aria-label="Show the camera again from {formatTimecode(h.r.start)} to {formatTimecode(h.r.end)}"
			disabled={session.busy}
			onpointerdown={(e) => e.stopPropagation()}
			onclick={() => session.showCameraAt(h.r.start)}
		></button>
	{/each}
	<span class="label pointer-events-none absolute top-0 left-1 truncate text-[11px] leading-4 text-subtle">
		{dropped ? 'Camera (left out)' : 'Camera'}
	</span>
</div>

<style>
	.filmed {
		position: absolute;
		top: 6px;
		bottom: 6px;
		border-radius: 2px;
		background: var(--text-subtle);
		pointer-events: none;
	}
	.dropped .filmed {
		background: var(--line-control);
	}
	.hidden-range {
		position: absolute;
		top: 2px;
		bottom: 2px;
		border: 1px solid var(--text-muted);
		border-radius: 2px;
		background: repeating-linear-gradient(135deg, rgb(10 10 10 / 0.9) 0 4px, rgb(60 60 60 / 0.9) 4px 8px);
		cursor: pointer;
	}
	.hidden-range:focus-visible {
		outline: 2px solid var(--focus);
		outline-offset: 1px;
	}
	.label {
		max-width: 40%;
		padding: 0 4px;
		border-radius: 2px;
		background: rgb(10 10 10 / 0.7);
	}
</style>
