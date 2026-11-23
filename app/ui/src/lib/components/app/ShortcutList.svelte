<script lang="ts">
	import { Kbd } from '$lib/components/ui/kbd';
	import type { Shortcut } from '$lib/format/shortcuts';

	/** Controls as rows of keycaps and what they do. */
	let { items, dense = false }: { items: Shortcut[]; dense?: boolean } = $props();
</script>

<dl class="grid grid-cols-[minmax(0,auto)_1fr] items-baseline {dense ? 'gap-x-4 gap-y-2' : 'gap-x-6 gap-y-3'} text-xs">
	{#each items as item (item.what)}
		<dt class="flex flex-wrap items-center gap-1">
			{#each item.combos as combo, c (c)}
				{#if c > 0}<span class="px-0.5 text-subtle">or</span>{/if}
				{#each combo as part, i (i)}
					{#if i > 0}<span class="text-subtle" aria-hidden="true">+</span>{/if}
					<!-- Keys separated by two spaces ("J  K  L") are a set: one cap each. -->
					{#each part.split(/\s{2,}/) as key, k (k)}
						<Kbd class="bg-selected text-foreground">{key}</Kbd>
					{/each}
				{/each}
			{/each}
		</dt>
		<dd class="text-muted-foreground">{item.what}</dd>
	{/each}
</dl>
