<script lang="ts">
	import type { Snippet } from 'svelte';

	let { id, title, action, children }: { id: string; title: string; action?: Snippet; children: Snippet } = $props();
</script>

<section class="flex flex-col gap-2 border-b border-line px-3 py-3" aria-labelledby={id}>
	<div class="flex min-h-5 items-center justify-between gap-2">
		<h2 {id} class="text-xs font-medium text-muted-foreground">{title}</h2>
		{@render action?.()}
	</div>
	{@render children()}
</section>
