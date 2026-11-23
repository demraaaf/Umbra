<script lang="ts">
	import type { SavingClip } from '$lib/stores/app.svelte';
	import { Spinner } from '$lib/components/ui/spinner';
	import { gameLabel } from '$lib/format/format';

	/**
	 * A clip being saved: from the hotkey until the file is there. While Umbra keeps recording
	 * the seconds after the hotkey it counts them down, then it is written (a moment).
	 */
	let { item }: { item: SavingClip } = $props();

	let now = $state(performance.now());
	$effect(() => {
		const timer = setInterval(() => (now = performance.now()), 250);
		return () => clearInterval(timer);
	});
	const left = $derived(Math.max(0, Math.ceil(item.waitSecs - (now - item.started) / 1000)));
	const waiting = $derived(left > 0);
	const fraction = $derived(item.waitSecs > 0 ? Math.min(1, (now - item.started) / 1000 / item.waitSecs) : 1);
</script>

<div class="flex min-w-0 flex-col overflow-hidden rounded-md border border-line bg-chrome" role="group" aria-label="Clip being saved">
	<span class="shimmer relative block aspect-video w-full overflow-hidden" aria-hidden="true">
		<span class="absolute top-2 left-2 flex items-center gap-1.5 rounded-xs bg-black/80 px-1.5 py-0.5 text-xs font-medium text-foreground">
			<Spinner class="size-3" />
			{waiting ? `Recording ${left} s more` : 'Saving clip'}
		</span>
		<span class="tnum absolute right-1 bottom-2 rounded-xs bg-black/80 px-1 text-xs font-medium text-foreground">{item.lengthSecs} s</span>
		{#if item.waitSecs > 0}
			<span class="bar absolute inset-x-0 bottom-0"><span style:transform="scaleX({fraction})"></span></span>
		{/if}
	</span>
	<span class="flex min-w-0 flex-col gap-0.5 border-t border-line px-2.5 pt-2 pb-2.5">
		<span class="truncate text-sm font-medium text-foreground">Clip of {gameLabel(item.game === 'Desktop' ? null : item.game)}</span>
		<span class="truncate text-xs text-muted-foreground" aria-live="polite">
			{waiting ? 'Keeps what happens right after the hotkey' : 'Writing the last ' + item.lengthSecs + ' s'}
		</span>
	</span>
</div>

<style>
	.shimmer {
		background: var(--surface-raised);
	}
	.shimmer::before {
		content: '';
		position: absolute;
		inset: 0;
		background: linear-gradient(100deg, transparent 30%, color-mix(in srgb, var(--text) 6%, transparent) 50%, transparent 70%);
		transform: translateX(-100%);
		animation: sheen 1.8s ease-in-out infinite;
	}
	@keyframes sheen {
		to {
			transform: translateX(100%);
		}
	}
	.bar {
		height: 3px;
		background: color-mix(in srgb, var(--text) 10%, transparent);
	}
	.bar > span {
		position: absolute;
		inset: 0;
		transform-origin: left;
		background: var(--text);
		transition: transform 250ms linear;
	}
	@media (prefers-reduced-motion: reduce) {
		.shimmer::before {
			animation: none;
		}
		.bar > span {
			transition: none;
		}
	}
</style>
