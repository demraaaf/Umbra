<script lang="ts">
	import * as Tooltip from '$lib/components/ui/tooltip';
	import { cn } from '$lib/utils';

	/** A term with a dotted underline; hover or focus it to read what it means. */
	let { label, explain, class: className }: { label: string; explain: string; class?: string } = $props();
	const id = $props.id();
</script>

<Tooltip.Root>
	<Tooltip.Trigger>
		{#snippet child({ props })}
			<button
				{...props}
				type="button"
				class={cn('term cursor-help rounded-xs text-left', className)}
				aria-describedby="{id}-explain"
			>
				{label}
			</button>
			<span id="{id}-explain" class="sr-only">{explain}</span>
		{/snippet}
	</Tooltip.Trigger>
	<Tooltip.Content class="max-w-64 text-xs leading-4">{explain}</Tooltip.Content>
</Tooltip.Root>

<style>
	.term {
		text-decoration: underline dotted;
		text-decoration-color: var(--text-subtle);
		text-underline-offset: 3px;
	}
</style>
