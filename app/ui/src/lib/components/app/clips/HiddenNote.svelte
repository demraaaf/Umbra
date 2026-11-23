<script lang="ts">
	import { view } from '$lib/stores/view.svelte';

	/** "N hidden in albums", with a switch to show them. Sits under the toolbar or selection bar. */
	let { count }: { count: number } = $props();
</script>

{#if count > 0 || view.showHidden}
	<div class="flex h-8 shrink-0 items-center gap-2 border-b border-line px-4 text-xs text-muted-foreground">
		{#if view.showHidden}
			<span>Showing clips from hidden albums.</span>
		{:else}
			<span class="tnum">{count} hidden in albums.</span>
		{/if}
		<button type="button" class="link" onclick={() => (view.showHidden = !view.showHidden)}>
			{view.showHidden ? 'Hide them again' : 'Show them'}
		</button>
	</div>
{/if}

<style>
	.link {
		color: var(--text);
		text-decoration: underline;
		text-underline-offset: 3px;
		text-decoration-color: var(--text-subtle);
		border-radius: 2px;
	}
	.link:hover {
		text-decoration-color: var(--text);
	}
</style>
