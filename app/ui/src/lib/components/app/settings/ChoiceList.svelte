<script lang="ts">
	import * as RadioGroup from '$lib/components/ui/radio-group';
	import { Label } from '$lib/components/ui/label';
	import { cn } from '$lib/utils';

	export interface Choice {
		value: string;
		label: string;
		detail?: string;
		badge?: string;
		disabled?: boolean;
	}

	/** A single-choice list drawn as dense rows, for monitors, windows and recording modes. */
	let {
		choices,
		value = $bindable(),
		name,
		labelledBy,
		class: className
	}: {
		choices: Choice[];
		value: string | undefined;
		name: string;
		labelledBy: string;
		class?: string;
	} = $props();
</script>

<RadioGroup.Root bind:value aria-labelledby={labelledBy} class={cn('gap-0 overflow-hidden rounded-md border border-line-control', className)}>
	{#each choices as choice, i (choice.value)}
		<Label
			for="{name}-{i}"
			class="choice flex min-h-8 cursor-pointer items-center gap-3 px-3 py-2 font-normal {i > 0 ? 'border-t border-line' : ''} {choice.disabled ? 'opacity-50' : ''}"
		>
			<RadioGroup.Item value={choice.value} id="{name}-{i}" disabled={choice.disabled} />
			<span class="flex min-w-0 flex-1 flex-col">
				<span class="truncate text-sm text-foreground">{choice.label}</span>
				{#if choice.detail}
					<span class="truncate text-xs text-muted-foreground">{choice.detail}</span>
				{/if}
			</span>
			{#if choice.badge}
				<span class="shrink-0 rounded-sm bg-selected px-1 text-xs text-muted-foreground">{choice.badge}</span>
			{/if}
		</Label>
	{/each}
</RadioGroup.Root>

<style>
	:global(.choice) {
		transition: background-color var(--dur-fast) var(--ease-out);
	}
	:global(.choice:hover) {
		background: var(--surface-raised);
	}
	:global(.choice:has([data-checked])),
	:global(.choice:has([aria-checked='true'])) {
		background: var(--surface-selected);
	}
</style>
