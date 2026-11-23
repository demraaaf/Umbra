<script lang="ts">
	import { tick } from 'svelte';
	import { Button } from '$lib/components/ui/button';
	import * as Dialog from '$lib/components/ui/dialog';
	import { Input } from '$lib/components/ui/input';
	import { Label } from '$lib/components/ui/label';
	import { Spinner } from '$lib/components/ui/spinner';

	/** Asks for an album name, for creating or renaming. */
	let {
		open = $bindable(false),
		title,
		confirmLabel,
		initial = '',
		onsubmit
	}: {
		open: boolean;
		title: string;
		confirmLabel: string;
		initial?: string;
		/** Resolves to true when done, so the dialog can close. */
		onsubmit: (name: string) => Promise<boolean>;
	} = $props();

	let name = $state('');
	let busy = $state(false);
	let input = $state<HTMLInputElement | null>(null);

	$effect(() => {
		if (open) {
			name = initial;
			void tick().then(() => input?.select());
		}
	});

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		const clean = name.trim();
		if (!clean || busy) return;
		busy = true;
		const ok = await onsubmit(clean);
		busy = false;
		if (ok) open = false;
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Content class="sm:max-w-sm">
		<form class="flex flex-col gap-4" onsubmit={submit}>
			<Dialog.Header>
				<Dialog.Title>{title}</Dialog.Title>
			</Dialog.Header>
			<div class="flex flex-col gap-2">
				<Label for="album-name" class="text-xs text-muted-foreground">Name</Label>
				<Input id="album-name" bind:ref={input} bind:value={name} maxlength={80} autocomplete="off" spellcheck={false} />
			</div>
			<Dialog.Footer>
				<Button type="button" variant="outline" onclick={() => (open = false)}>Cancel</Button>
				<Button type="submit" disabled={!name.trim() || busy}>
					{#if busy}<Spinner data-icon="inline-start" />{/if}
					{confirmLabel}
				</Button>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>
