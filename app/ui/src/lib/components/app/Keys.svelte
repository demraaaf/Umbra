<script lang="ts">
	import { Kbd } from '$lib/components/ui/kbd';

	/**
	 * A key combination drawn as keycaps, read out as one phrase. `all` draws several
	 * bindings of one action, separated by "or".
	 */
	let { parts = [], all, label }: { parts?: string[]; all?: string[][]; label: string } = $props();

	const groups = $derived(all && all.length > 0 ? all : [parts]);
</script>

<span
	class="flex min-w-0 flex-wrap items-center justify-center gap-1 text-xs"
	aria-label="{label}: {groups.map((g) => g.join(' plus ')).join(' or ')}"
>
	{#each groups as group, g (g)}
		{#if g > 0}<span class="px-0.5 text-subtle" aria-hidden="true">or</span>{/if}
		{#each group as part, i (i)}
			{#if i > 0}<span class="text-subtle" aria-hidden="true">+</span>{/if}
			<Kbd class="bg-selected text-muted-foreground" aria-hidden="true">{part}</Kbd>
		{/each}
	{/each}
</span>
