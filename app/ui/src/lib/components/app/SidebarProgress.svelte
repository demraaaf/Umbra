<script lang="ts">
	import * as Tooltip from '$lib/components/ui/tooltip';

	/**
	 * Work that runs on while you do other things (an import, an export), at the bottom of the
	 * sidebar: a ring with the percent, a line of detail, and a click to see it.
	 */
	let {
		label,
		percent,
		detail,
		tooltip,
		collapsed,
		onclick
	}: { label: string; percent: number; detail: string; tooltip: string; collapsed: boolean; onclick: () => void } = $props();
</script>

<Tooltip.Root disabled={!collapsed}>
	<Tooltip.Trigger>
		{#snippet child({ props })}
			<button {...props} type="button" class="progress-item" class:collapsed aria-label="{tooltip}. Show details" {onclick}>
				<svg class="progress-ring" viewBox="0 0 16 16" aria-hidden="true">
					<circle class="track" cx="8" cy="8" r="6" />
					<circle class="value" cx="8" cy="8" r="6" pathLength="100" stroke-dasharray="{percent} 100" />
				</svg>
				<span class="label flex min-w-0 flex-1 flex-col">
					<span class="flex items-baseline gap-2">
						<span class="truncate text-foreground">{label}</span>
						<span class="tnum ml-auto text-xs text-muted-foreground">{percent}%</span>
					</span>
					<span class="tnum truncate text-left text-xs text-subtle">{detail}</span>
				</span>
			</button>
		{/snippet}
	</Tooltip.Trigger>
	<Tooltip.Content side="right">{tooltip}</Tooltip.Content>
</Tooltip.Root>

<style>
	.progress-item {
		display: flex;
		width: 100%;
		align-items: center;
		gap: 10px;
		border-radius: 4px;
		padding: 6px 8px;
		font-size: 13px;
		transition:
			background-color var(--dur-fast) var(--ease-out),
			transform var(--dur-fast) var(--ease-out);
	}
	.progress-item:hover {
		background: var(--surface-raised);
	}
	.progress-item:active {
		transform: scale(0.98);
	}
	.progress-item:focus-visible {
		outline: 2px solid var(--focus);
		outline-offset: -2px;
	}
	.progress-ring {
		width: 16px;
		height: 16px;
		flex-shrink: 0;
		transform: rotate(-90deg);
	}
	.progress-ring circle {
		fill: none;
		stroke-width: 2;
	}
	.progress-ring .track {
		stroke: var(--line-control);
	}
	.progress-ring .value {
		stroke: var(--text);
		stroke-linecap: round;
		transition: stroke-dasharray 480ms linear;
	}
	.progress-item.collapsed {
		justify-content: center;
		padding: 6px 0;
	}
	.progress-item.collapsed .label {
		display: none;
	}
	@media (max-width: 879px) {
		.progress-item {
			justify-content: center;
			padding: 6px 0;
		}
		.label {
			display: none;
		}
	}
</style>
