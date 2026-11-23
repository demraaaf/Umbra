<script lang="ts">
	import { tick } from 'svelte';
	import { Input } from '$lib/components/ui/input';
	import { formatTimecode, parseTimecode } from '$lib/format/format';
	import { cn } from '$lib/utils';

	/** A timecode field (`m:ss.mmm`). Accepts `12`, `12.5`, `1:02` and so on; commits on Enter or blur. */
	let {
		value,
		label,
		id,
		disabled = false,
		class: className,
		oncommit
	}: {
		value: number;
		label: string;
		id?: string;
		disabled?: boolean;
		class?: string;
		oncommit: (seconds: number) => void;
	} = $props();

	let text = $state('');
	let focused = $state(false);

	$effect(() => {
		if (!focused) text = formatTimecode(value);
	});

	function commit() {
		const parsed = parseTimecode(text);
		if (parsed === null) {
			text = formatTimecode(value);
			return;
		}
		oncommit(parsed);
		// The parent may snap or clamp; show whatever it settled on.
		void tick().then(() => (text = formatTimecode(value)));
	}
</script>

<Input
	{id}
	type="text"
	inputmode="decimal"
	aria-label={label}
	autocomplete="off"
	spellcheck={false}
	{disabled}
	class={cn('tnum text-right', className)}
	value={text}
	oninput={(e) => (text = e.currentTarget.value)}
	onfocus={(e) => {
		focused = true;
		e.currentTarget.select();
	}}
	onblur={() => {
		focused = false;
		commit();
		text = formatTimecode(value);
	}}
	onkeydown={(e) => {
		if (e.key === 'Enter') {
			e.preventDefault();
			commit();
			e.currentTarget.select();
		} else if (e.key === 'Escape') {
			text = formatTimecode(value);
			e.currentTarget.select();
		}
	}}
/>
