<script lang="ts">
	import * as InputGroup from '$lib/components/ui/input-group';
	import { cn } from '$lib/utils';

	/**
	 * A typed whole-number field. Text is only validated when committed (Enter or blur), so
	 * typing "1" on the way to "144" never gets clamped mid-edit. Arrow keys step the value.
	 */
	let {
		value = $bindable(0),
		min = 0,
		max = Number.MAX_SAFE_INTEGER,
		step = 1,
		unit,
		even = false,
		disabled = false,
		id,
		label,
		class: className,
		oncommit
	}: {
		value: number;
		min?: number;
		max?: number;
		step?: number;
		unit?: string;
		/** Round to an even number, which video encoders require for sizes and offsets. */
		even?: boolean;
		disabled?: boolean;
		id?: string;
		label: string;
		class?: string;
		oncommit?: (value: number) => void;
	} = $props();

	let text = $state('');
	let focused = $state(false);

	$effect(() => {
		if (!focused) text = String(value);
	});

	function clean(v: number): number {
		let next = Math.round(v);
		if (even) next = Math.round(next / 2) * 2;
		return Math.min(max, Math.max(min, next));
	}

	function commit() {
		const parsed = Number(text.replace(/[\s,_]/g, ''));
		if (text.trim() === '' || !Number.isFinite(parsed)) {
			text = String(value);
			return;
		}
		const next = clean(parsed);
		value = next;
		text = String(next);
		oncommit?.(next);
	}

	function onkeydown(e: KeyboardEvent) {
		if (e.key === 'Enter') {
			e.preventDefault();
			commit();
			(e.currentTarget as HTMLInputElement).select();
		} else if (e.key === 'Escape') {
			text = String(value);
			(e.currentTarget as HTMLInputElement).select();
		} else if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
			e.preventDefault();
			const delta = (e.key === 'ArrowUp' ? 1 : -1) * step * (e.shiftKey ? 10 : 1);
			const base = Number(text.replace(/[\s,_]/g, ''));
			const next = clean((Number.isFinite(base) ? base : value) + delta);
			value = next;
			text = String(next);
			oncommit?.(next);
		}
	}
</script>

<InputGroup.Root class={cn('w-full', className)}>
	<InputGroup.Input
		{id}
		type="text"
		inputmode="numeric"
		role="spinbutton"
		aria-label={label}
		aria-valuenow={value}
		aria-valuemin={min}
		aria-valuemax={max === Number.MAX_SAFE_INTEGER ? undefined : max}
		autocomplete="off"
		spellcheck={false}
		{disabled}
		class="tnum text-right"
		value={text}
		oninput={(e) => (text = e.currentTarget.value)}
		onfocus={(e) => {
			focused = true;
			e.currentTarget.select();
		}}
		onblur={() => {
			focused = false;
			commit();
		}}
		{onkeydown}
	/>
	{#if unit}
		<InputGroup.Addon align="inline-end">
			<InputGroup.Text class="text-xs">{unit}</InputGroup.Text>
		</InputGroup.Addon>
	{/if}
</InputGroup.Root>
