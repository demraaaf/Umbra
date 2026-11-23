<script lang="ts">
	import { Switch } from '$lib/components/ui/switch';
	import { Label } from '$lib/components/ui/label';
	import { app, patchSettings } from '$lib/stores/app.svelte';
	import { bindingLabels } from '$lib/format/hotkey';
	import Keys from '../Keys.svelte';
	import PanelSection from './PanelSection.svelte';

	const input = $derived(app.settings?.input);
	const status = $derived(app.status?.input);
	const on = $derived(status?.on ?? false);

	const stateText = $derived.by(() => {
		if (!input?.enabled) return 'Off';
		if (!on) return 'Starting';
		if (status?.guard === 'password') return 'Password field';
		return status?.rule ? 'Recording' : 'Nothing kept';
	});
</script>

<PanelSection id="panel-input" title="Keyboard and mouse">
	{#snippet action()}
		<span class="flex items-center gap-1 text-xs whitespace-nowrap {on ? 'text-foreground' : 'text-subtle'}" role="status" aria-live="polite">
			<span class="dot" class:live={on && !!status?.rule} aria-hidden="true"></span>
			{stateText}
		</span>
	{/snippet}
	{#if input}
		<div class="flex items-center justify-between gap-2">
			<Label for="panel-input-on" class="min-w-0 truncate text-xs font-normal text-foreground">Record keys and buttons</Label>
			<Switch id="panel-input-on" size="sm" checked={input.enabled} onCheckedChange={(v) => patchSettings((s) => (s.input.enabled = v))} />
		</div>
		{#if on && status?.app}
			<p class="truncate text-xs text-muted-foreground" title="{status.app} → {status.rule ?? 'nothing kept'}">
				<span class="text-foreground">{status.app}</span> → {status.rule ?? 'nothing kept'}{status.guard === 'chat' ? ' (chat)' : ''}
			</p>
		{/if}
		{#if input.hotkey}
			<div class="flex items-center justify-between gap-2 text-xs">
				<span class="text-muted-foreground">On or off</span>
				<Keys all={bindingLabels(input.hotkey)} label="Keyboard and mouse hotkey" />
			</div>
		{/if}
	{/if}
</PanelSection>

<style>
	.dot {
		width: 8px;
		height: 8px;
		border-radius: 999px;
		border: 1px solid var(--text-subtle);
	}
	.dot.live {
		background: var(--text);
		border-color: var(--text);
	}
</style>
