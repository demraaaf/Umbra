<script lang="ts">
	/** A file path that wraps only after its separators, never in the middle of a name. */
	let { path }: { path: string } = $props();

	const parts = $derived(path.split(/(?<=[\/])/));
</script>

{#each parts as part, i (i)}{part}<wbr />{/each}
