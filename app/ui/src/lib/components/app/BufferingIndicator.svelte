<script lang="ts">
	import { Spinner } from '$lib/components/ui/spinner';

	/**
	 * Over the picture while playback waits for data after a jump. It shows only after a
	 * moment, so the usual quick jump does not flash it.
	 */
	let { active }: { active: boolean } = $props();

	let shown = $state(false);
	$effect(() => {
		if (!active) {
			shown = false;
			return;
		}
		const timer = setTimeout(() => (shown = true), 180);
		return () => clearTimeout(timer);
	});
</script>

{#if shown}
	<div class="pointer-events-none absolute inset-0 flex items-center justify-center" role="status" aria-label="Loading this part">
		<span class="flex size-11 items-center justify-center rounded-full bg-black/70">
			<Spinner class="size-5 text-foreground" />
		</span>
	</div>
{/if}
