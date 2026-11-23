<script lang="ts">
	import type { Component, Snippet } from 'svelte';
	import { Button, type ButtonVariant } from '$lib/components/ui/button';
	import { Kbd } from '$lib/components/ui/kbd';
	import * as Tooltip from '$lib/components/ui/tooltip';
	import { cn } from '$lib/utils';

	/** A button that shows an icon, names itself for screen readers and explains itself in a tooltip. */
	let {
		label,
		shortcut,
		icon: Icon,
		onclick,
		pressed,
		disabled = false,
		variant = 'ghost',
		size = 'icon-sm',
		class: className,
		children
	}: {
		label: string;
		shortcut?: string;
		icon?: Component<{ class?: string }>;
		onclick?: (event: MouseEvent) => void;
		pressed?: boolean;
		disabled?: boolean;
		variant?: ButtonVariant;
		size?: 'icon' | 'icon-sm' | 'icon-xs' | 'sm' | 'default';
		class?: string;
		children?: Snippet;
	} = $props();
</script>

<Tooltip.Root>
	<Tooltip.Trigger>
		{#snippet child({ props })}
			<Button
				{...props}
				{variant}
				{size}
				{disabled}
				{onclick}
				aria-label={label}
				aria-pressed={pressed}
				aria-keyshortcuts={shortcut}
				class={cn(pressed && 'bg-on text-foreground', className)}
			>
				{#if Icon}<Icon />{/if}
				{@render children?.()}
			</Button>
		{/snippet}
	</Tooltip.Trigger>
	<Tooltip.Content>
		{label}
		{#if shortcut}<Kbd>{shortcut}</Kbd>{/if}
	</Tooltip.Content>
</Tooltip.Root>
