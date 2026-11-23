<script lang="ts">
	import ImportIcon from '@lucide/svelte/icons/import';
	import { importer } from '$lib/stores/importer.svelte';

	/** Over the whole window while files are dragged onto it from Explorer. */
	const count = $derived(importer.dragging?.length ?? 0);
	const folders = $derived(importer.dragging?.filter((p) => !/\.[a-z0-9]{2,5}$/i.test(p)).length ?? 0);
	const what = $derived(
		folders === count
			? `${count} ${count === 1 ? 'folder' : 'folders'}`
			: folders === 0
				? `${count} ${count === 1 ? 'file' : 'files'}`
				: `${count} items`
	);
</script>

{#if importer.dragging}
	<div class="overlay pointer-events-none fixed inset-0 z-50 flex items-center justify-center p-6" role="status" aria-live="polite">
		<div class="frame flex size-full flex-col items-center justify-center gap-3 rounded-lg text-center">
			<span class="flex size-12 items-center justify-center rounded-full bg-surface-raised">
				<ImportIcon class="size-6 text-foreground" aria-hidden="true" />
			</span>
			<p class="text-base font-medium text-foreground">Drop to import {what}</p>
			<p class="max-w-sm text-sm text-muted-foreground">
				Videos are copied into your clips folder by game. You can check what comes in before anything is copied.
			</p>
		</div>
	</div>
{/if}

<style>
	.overlay {
		background: color-mix(in srgb, var(--background) 82%, transparent);
		animation: in 120ms var(--ease-out);
	}
	.frame {
		border: 2px dashed var(--line-strong);
	}
	@keyframes in {
		from {
			opacity: 0;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.overlay {
			animation: none;
		}
	}
</style>
