<script lang="ts">
	import PlusIcon from '@lucide/svelte/icons/plus';
	import XIcon from '@lucide/svelte/icons/x';
	import { Button } from '$lib/components/ui/button';
	import { Kbd } from '$lib/components/ui/kbd';
	import {
		bindingsOf,
		comboFromKeyboardEvent,
		comboFromMouseEvent,
		isTypingKey,
		keyLabel,
		splitCombo
	} from '$lib/format/hotkey';

	/**
	 * Records an action's hotkeys. Each binding is a chip: click it and press a key combination
	 * or a mouse button (middle, back or forward) to change it, or remove it with its ×. The +
	 * adds another binding, so for example push-to-talk can be on both side mouse buttons. The
	 * value is the bindings joined by commas, in the `Alt+F10` / `Mouse4` format from types.ts.
	 */
	let {
		value = $bindable(),
		defaultValue,
		labelledBy,
		disabled = false
	}: { value: string; defaultValue?: string; labelledBy: string; disabled?: boolean } = $props();

	const MAX = 4;
	const uid = $props.id();

	const bindings = $derived(bindingsOf(value));
	/** The binding being recorded: an index, or `bindings.length` for a new one; null when idle. */
	let target = $state<number | null>(null);
	const recording = $derived(target !== null);
	let group = $state<HTMLDivElement | null>(null);
	let announcement = $state('');

	const labelsOf = (combo: string) => splitCombo(combo).map(keyLabel);
	const spoken = (combo: string) => labelsOf(combo).join(' plus ');

	function commit(combo: string) {
		const index = target ?? bindings.length;
		const next = [...bindings];
		// Recording a binding the action already has just keeps the one.
		const existing = next.indexOf(combo);
		if (existing >= 0 && existing !== index) {
			if (index < next.length) next.splice(index, 1);
		} else {
			next[index] = combo;
		}
		value = next.join(', ');
		announcement =
			existing >= 0 && existing !== index ? `${spoken(combo)} is already set.` : `Hotkey set to ${spoken(combo)}.`;
		stop();
	}

	function stop() {
		const index = target;
		target = null;
		// Back to the chip that was recorded (or the + after adding).
		queueMicrotask(() => {
			const chips = group?.querySelectorAll<HTMLButtonElement>('[data-chip]');
			chips?.[Math.min(index ?? 0, chips.length - 1)]?.focus();
		});
	}

	function remove(index: number) {
		value = bindings.filter((_, i) => i !== index).join(', ');
		announcement = 'Hotkey removed.';
	}

	$effect(() => {
		if (!recording) return;
		announcement = 'Recording. Press a key combination or a mouse button. Escape cancels.';

		const onKey = (e: KeyboardEvent) => {
			const bare = !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey;
			// Tab leaves the field, so a keyboard user is never trapped in recording mode.
			if (e.code === 'Tab' && bare) {
				target = null;
				return;
			}
			e.preventDefault();
			e.stopPropagation();
			if (e.repeat) return;
			if (e.code === 'Escape' && bare) {
				announcement = 'Cancelled.';
				stop();
				return;
			}
			const combo = comboFromKeyboardEvent(e);
			if (combo) commit(combo);
		};

		const onMouse = (e: MouseEvent) => {
			if (e.button === 0 || e.button === 2) {
				// An ordinary click outside cancels; clicks inside are the chips' own.
				if (!group?.contains(e.target as Node) && e.type === 'mousedown') target = null;
				return;
			}
			const combo = comboFromMouseEvent(e);
			if (!combo) return;
			e.preventDefault();
			e.stopPropagation();
			if (e.type === 'mousedown') commit(combo);
		};

		window.addEventListener('keydown', onKey, true);
		window.addEventListener('mousedown', onMouse, true);
		window.addEventListener('mouseup', onMouse, true);
		window.addEventListener('auxclick', onMouse, true);
		return () => {
			window.removeEventListener('keydown', onKey, true);
			window.removeEventListener('mousedown', onMouse, true);
			window.removeEventListener('mouseup', onMouse, true);
			window.removeEventListener('auxclick', onMouse, true);
		};
	});
</script>

<div class="flex items-start gap-2">
	<div bind:this={group} class="flex min-w-0 flex-1 flex-wrap items-center gap-1.5" role="group" aria-labelledby={labelledBy} aria-describedby="{uid}-help">
		{#each bindings as combo, i (combo)}
			<span class="chip" class:recording={target === i}>
				<button
					type="button"
					data-chip
					class="chip-key"
					{disabled}
					aria-pressed={target === i}
					aria-label={target === i ? 'Recording: press keys or a mouse button' : `${spoken(combo)}. Click to change.`}
					onclick={() => (target = target === i ? null : i)}
					onblur={() => target === i && (target = null)}
				>
					{#if target === i}
						<span class="text-muted-foreground">Press keys or a button</span>
					{:else}
						{#each labelsOf(combo) as part, k (k)}
							{#if k > 0}<span class="text-subtle" aria-hidden="true">+</span>{/if}
							<Kbd class="bg-selected text-foreground">{part}</Kbd>
						{/each}
					{/if}
				</button>
				{#if bindings.length > 1 && target !== i}
					<button type="button" class="chip-remove" {disabled} aria-label="Remove {spoken(combo)}" title="Remove" onclick={() => remove(i)}>
						<XIcon class="size-3" aria-hidden="true" />
					</button>
				{/if}
			</span>
		{/each}
		{#if target === bindings.length}
			<span class="chip recording">
				<span class="chip-key text-muted-foreground" role="status">Press keys or a button</span>
			</span>
		{:else if bindings.length < MAX}
			<button
				type="button"
				data-chip
				class="add"
				{disabled}
				aria-label="Add another key or button for this action"
				title="Add another key or button"
				onclick={() => (target = bindings.length)}
			>
				<PlusIcon class="size-3.5" aria-hidden="true" />
			</button>
		{/if}
	</div>
	{#if defaultValue}
		<Button variant="ghost" size="sm" onclick={() => defaultValue && (value = defaultValue)} disabled={value === defaultValue || recording || disabled}>Reset</Button>
	{/if}
</div>
<p id="{uid}-help" class="mt-2 text-xs text-subtle">
	{#if recording}
		Esc cancels. Middle, back and forward mouse buttons work too.
	{:else if isTypingKey(value)}
		Without Ctrl, Alt or Shift this key also triggers while you type in a game.
	{:else}
		Click a key to change it; + adds another, so either one works.
	{/if}
</p>
<span class="sr-only" role="status" aria-live="polite">{announcement}</span>

<style>
	.chip {
		display: inline-flex;
		height: 28px;
		align-items: stretch;
		border-radius: 4px;
		border: 1px solid var(--line-control);
		background: rgb(64 64 64 / 0.3);
		transition:
			border-color var(--dur-base) var(--ease-out),
			background-color var(--dur-base) var(--ease-out);
	}
	.chip:hover:not(:has(:disabled)) {
		background: rgb(64 64 64 / 0.5);
	}
	.chip.recording {
		border-color: var(--focus);
		background: var(--surface-raised);
	}
	.chip-key {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		padding: 0 8px;
		font-size: 13px;
		border-radius: 3px;
	}
	.chip-remove {
		display: inline-flex;
		align-items: center;
		padding: 0 6px 0 2px;
		color: var(--text-subtle);
		border-radius: 3px;
		transition: color var(--dur-fast) var(--ease-out);
	}
	.chip-remove:hover:not(:disabled) {
		color: var(--text);
	}
	.add {
		display: inline-flex;
		height: 28px;
		width: 28px;
		align-items: center;
		justify-content: center;
		border-radius: 4px;
		border: 1px dashed var(--line-control);
		color: var(--text-muted);
		transition:
			border-color var(--dur-fast) var(--ease-out),
			color var(--dur-fast) var(--ease-out);
	}
	.add:hover:not(:disabled) {
		border-color: var(--text-subtle);
		color: var(--text);
	}
	.chip-key:focus-visible,
	.chip-remove:focus-visible,
	.add:focus-visible {
		outline: 2px solid var(--focus);
		outline-offset: 2px;
	}
	button:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}
</style>
