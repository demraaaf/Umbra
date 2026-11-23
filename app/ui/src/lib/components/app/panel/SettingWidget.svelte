<script lang="ts">
	import type { SettingDef } from '$lib/panel/widgets';
	import * as ToggleGroup from '$lib/components/ui/toggle-group';
	import * as Select from '$lib/components/ui/select';
	import { Slider } from '$lib/components/ui/slider';
	import NumberField from '../fields/NumberField.svelte';
	import { app, patchSettings } from '$lib/stores/app.svelte';

	/**
	 * A setting in the panel. Changing it saves it at once, as the same setting in Settings
	 * would after Apply. A switch is a tile that lights up when on, two to a row.
	 */
	let { def }: { def: SettingDef } = $props();

	const settings = $derived(app.settings);
	let saving = $state(false);

	async function change(apply: (s: NonNullable<typeof app.settings>) => void) {
		if (saving) return;
		saving = true;
		await patchSettings(apply);
		saving = false;
	}
</script>

{#if settings}
	{#if def.type === 'toggle'}
		{@const on = def.get(settings)}
		<button
			type="button"
			class="tile flex h-full w-full flex-col items-start gap-2 rounded-lg p-2.5 text-left"
			class:on
			role="switch"
			aria-checked={on}
			disabled={saving}
			title={def.description}
			onclick={() => change((s) => def.set(s, !on))}
		>
			<span class="icon flex size-7 items-center justify-center rounded-full">
				<def.icon class="size-4" />
			</span>
			<span class="flex min-w-0 flex-col">
				<span class="truncate text-xs font-medium">{def.label}</span>
				<span class="state text-xs">{on ? 'On' : 'Off'}</span>
			</span>
		</button>
	{:else}
		<section class="flex flex-col gap-2 border-b border-line px-3 py-3" aria-label={def.label} title={def.description}>
			<div class="flex min-h-5 items-center gap-2">
				<def.icon class="size-3.5 text-muted-foreground" aria-hidden="true" />
				<h2 class="text-xs font-medium text-muted-foreground">{def.label}</h2>
				{#if def.type === 'slider'}
					<span class="tnum ml-auto text-xs text-foreground">{def.format(def.get(settings))}</span>
				{/if}
			</div>
			{#if def.type === 'choice' && def.options.length <= 4}
				<ToggleGroup.Root
					type="single"
					variant="outline"
					size="sm"
					class="w-full"
					value={def.get(settings)}
					onValueChange={(v) => v && v !== def.get(settings) && change((s) => def.set(s, v))}
					disabled={saving}
					aria-label={def.label}
				>
					{#each def.options as option (option.value)}
						<ToggleGroup.Item value={option.value} class="flex-1 px-1 text-xs">{option.label}</ToggleGroup.Item>
					{/each}
				</ToggleGroup.Root>
			{:else if def.type === 'choice'}
				{@const current = def.get(settings)}
				<Select.Root type="single" value={current} onValueChange={(v) => v && v !== current && change((s) => def.set(s, v))}>
					<Select.Trigger size="sm" class="w-full" aria-label={def.label}>
						{def.options.find((o) => o.value === current)?.label ?? current}
					</Select.Trigger>
					<Select.Content>
						<Select.Group>
							{#each def.options as option (option.value)}
								<Select.Item value={option.value} label={option.label}>{option.label}</Select.Item>
							{/each}
							{#if !def.options.some((o) => o.value === current)}
								<Select.Item value={current} label={current}>{current} (set in Settings)</Select.Item>
							{/if}
						</Select.Group>
					</Select.Content>
				</Select.Root>
			{:else if def.type === 'number'}
				<NumberField
					label={def.label}
					value={def.get(settings)}
					min={def.min}
					max={def.max}
					step={def.step}
					unit={def.unit}
					disabled={saving}
					oncommit={(v) => v !== def.get(settings) && change((s) => def.set(s, v))}
				/>
			{:else if def.type === 'slider'}
				<Slider
					type="single"
					min={def.min}
					max={def.max}
					step={def.step}
					value={def.get(settings)}
					onValueCommit={(v: number) => change((s) => def.set(s, v))}
					aria-label={def.label}
				/>
			{/if}
		</section>
	{/if}
{/if}

<style>
	.tile {
		background: var(--surface-raised);
		color: var(--text);
		transition:
			background-color var(--dur-fast) var(--ease-out),
			color var(--dur-fast) var(--ease-out),
			transform var(--dur-fast) var(--ease-out);
	}
	.tile:hover:not(:disabled) {
		background: var(--surface-selected);
	}
	.tile:active:not(:disabled) {
		transform: scale(0.97);
	}
	.tile:focus-visible {
		outline: 2px solid var(--focus);
		outline-offset: 2px;
	}
	.icon {
		background: color-mix(in srgb, var(--text) 10%, transparent);
	}
	.state {
		color: var(--text-subtle);
	}
	/* On: the tile turns light, like a lit switch. */
	.tile.on {
		background: var(--text);
		color: var(--background);
	}
	.tile.on .icon {
		background: color-mix(in srgb, var(--background) 14%, transparent);
	}
	.tile.on .state {
		color: color-mix(in srgb, var(--background) 65%, transparent);
	}
</style>
