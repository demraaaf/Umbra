<script lang="ts">
	import type { Snippet } from 'svelte';
	import { Label } from '$lib/components/ui/label';
	import { settingsNav, showAdvanced } from '$lib/stores/settingsNav.svelte';

	/**
	 * One setting: what it is on the left, the control on the right. Below the container
	 * breakpoint the control drops under the label. Pass `controlId` when the control is a single
	 * input so the label is associated with it; otherwise the label names the group via `labelId`.
	 */
	let {
		label,
		description,
		controlId,
		labelId,
		children,
		below,
		advanced = false
	}: {
		label: string;
		description?: string;
		controlId?: string;
		labelId?: string;
		children: Snippet;
		/** Extra content spanning the full row under the control (warnings, previews). */
		below?: Snippet;
		/** Hidden unless advanced settings are on (or a search is running). */
		advanced?: boolean;
	} = $props();
</script>

{#if !advanced || showAdvanced()}
<div class="border-t border-line py-3" data-settings-row>
	<div class="grid grid-cols-1 gap-2 @lg:grid-cols-[minmax(0,1fr)_minmax(0,320px)] @lg:gap-6">
		<div class="min-w-0">
			{#if controlId}
				<Label for={controlId} class="text-sm font-medium text-foreground">{label}</Label>
			{:else}
				<div id={labelId} class="text-sm font-medium text-foreground">{label}</div>
			{/if}
			{#if advanced && !settingsNav.advanced}
				<span class="text-xs text-subtle">Advanced</span>
			{/if}
			{#if description}
				<p class="mt-1 text-xs text-muted-foreground">{description}</p>
			{/if}
		</div>
		<div class="min-w-0">
			{@render children()}
		</div>
	</div>
	{#if below}
		<div class="mt-2">
			{@render below()}
		</div>
	{/if}
</div>
{/if}
