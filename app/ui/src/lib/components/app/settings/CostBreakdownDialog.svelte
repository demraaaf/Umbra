<script lang="ts">
	import * as Dialog from '$lib/components/ui/dialog';
	import type { Cost } from '$lib/api/types';

	/** What each costly setting costs, from the estimate: why, what to change and what that gives up. */
	let { open = $bindable(false), costs }: { open?: boolean; costs: Cost[] } = $props();

	const RESOURCES: Record<Cost['resource'], string> = {
		encoder: 'Video encoder',
		gpu: 'Graphics card (3D)',
		cpu: 'Processor',
		memory: 'Memory',
		disk: 'Disk'
	};
	const IMPACTS: Record<Cost['impact'], string> = { high: 'High', medium: 'Medium', low: 'Low' };
</script>

<Dialog.Root bind:open>
	<Dialog.Content class="flex max-h-[88vh] flex-col sm:max-w-2xl">
		<Dialog.Header>
			<Dialog.Title>What costs what</Dialog.Title>
			<Dialog.Description>
				The video encoder is a separate part of the graphics card that games do not use, so its load rarely costs frames in a game.
				Task Manager's GPU column shows the busiest part of the card, which is often the encoder.
			</Dialog.Description>
		</Dialog.Header>

		<ol class="-mx-4 flex min-h-0 flex-col overflow-y-auto px-4">
			{#each costs as cost (cost.title)}
				<li class="flex flex-col gap-2 border-t border-line py-3 first:border-t-0">
					<div class="flex flex-wrap items-baseline gap-x-2 gap-y-1">
						<h3 class="text-sm font-medium text-foreground">{cost.title}</h3>
						<span class="text-xs text-muted-foreground">{RESOURCES[cost.resource]}</span>
						<span class="impact ml-auto" class:high={cost.impact === 'high'}>{IMPACTS[cost.impact]}<span class="sr-only"> impact</span></span>
					</div>
					<dl class="grid grid-cols-[6rem_minmax(0,1fr)] gap-x-3 gap-y-1.5 text-xs">
						<dt class="text-subtle">Why</dt>
						<dd class="text-foreground">{cost.why}</dd>
						<dt class="text-subtle">To lower it</dt>
						<dd class="text-foreground">{cost.change}</dd>
						<dt class="text-subtle">What you give up</dt>
						<dd class="text-muted-foreground">{cost.catch}</dd>
					</dl>
				</li>
			{/each}
		</ol>
	</Dialog.Content>
</Dialog.Root>

<style>
	.impact {
		border: 1px solid var(--line-strong);
		border-radius: 2px;
		padding: 0 4px;
		font-size: 12px;
		line-height: 16px;
		color: var(--text-muted);
	}
	.impact.high {
		border-color: var(--text);
		color: var(--text);
	}
</style>
