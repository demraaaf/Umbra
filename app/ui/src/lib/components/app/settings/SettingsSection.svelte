<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		id,
		title,
		description,
		page = id,
		children
	}: {
		id: string;
		title: string;
		description?: string;
		/** The Settings page it is on, when that holds more than one section. */
		page?: string;
		children: Snippet;
	} = $props();
</script>

<section {id} class="settings-section scroll-mt-4 pb-6" aria-labelledby="{id}-title" data-section={page}>
	<div class="pt-4 pb-2">
		<h2 id="{id}-title" class="text-base font-semibold text-foreground">{title}</h2>
		{#if description}
			<p class="mt-1 text-xs text-muted-foreground">{description}</p>
		{/if}
	</div>
	<div class="flex flex-col">
		{@render children()}
	</div>
</section>
