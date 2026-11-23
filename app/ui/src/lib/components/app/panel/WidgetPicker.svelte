<script lang="ts">
	import CheckIcon from '@lucide/svelte/icons/check';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import SearchIcon from '@lucide/svelte/icons/search';
	import * as Dialog from '$lib/components/ui/dialog';
	import * as InputGroup from '$lib/components/ui/input-group';
	import { Button } from '$lib/components/ui/button';
	import { BLOCKS, SETTINGS, SETTING_GROUPS } from '$lib/panel/widgets';

	/** Everything the panel can show, to add: its own parts, then any setting by group. */
	let {
		open = $bindable(false),
		added,
		onadd
	}: { open: boolean; added: string[]; onadd: (id: string) => void } = $props();

	let query = $state('');
	$effect(() => {
		if (open) query = '';
	});

	const q = $derived(query.trim().toLowerCase());
	const hit = (...texts: string[]) => !q || texts.some((t) => t.toLowerCase().includes(q));
	const blocks = $derived(BLOCKS.filter((b) => hit(b.title, b.description)));
	const groups = $derived(
		SETTING_GROUPS.map((group) => ({
			group,
			items: SETTINGS.filter((s) => s.group === group && hit(s.label, s.description, group))
		})).filter((g) => g.items.length > 0)
	);
</script>

{#snippet row(id: string, title: string, description: string, Icon: import('svelte').Component)}
	{@const has = added.includes(id)}
	<li class="flex items-center gap-3 rounded-md px-2 py-2 hover:bg-surface-raised">
		<span class="flex size-8 shrink-0 items-center justify-center rounded-md bg-surface-raised text-muted-foreground">
			<Icon class="size-4" />
		</span>
		<span class="flex min-w-0 flex-1 flex-col">
			<span class="truncate text-sm text-foreground">{title}</span>
			<span class="truncate text-xs text-subtle" {title}>{description}</span>
		</span>
		{#if has}
			<span class="flex shrink-0 items-center gap-1 text-xs text-subtle"><CheckIcon class="size-3.5" aria-hidden="true" /> Added</span>
		{:else}
			<Button variant="outline" size="sm" onclick={() => onadd(id)} aria-label="Add {title}">
				<PlusIcon data-icon="inline-start" />
				Add
			</Button>
		{/if}
	</li>
{/snippet}

<Dialog.Root bind:open>
	<Dialog.Content class="flex max-h-[85vh] flex-col sm:max-w-lg">
		<Dialog.Header>
			<Dialog.Title>Add to the panel</Dialog.Title>
			<Dialog.Description>The panel's own parts, or any setting to change it from there.</Dialog.Description>
		</Dialog.Header>
		<InputGroup.Root>
			<InputGroup.Addon><SearchIcon /></InputGroup.Addon>
			<InputGroup.Input placeholder="Search widgets and settings" bind:value={query} aria-label="Search widgets and settings" />
		</InputGroup.Root>
		<div class="-mx-4 min-h-[18rem] flex-1 overflow-y-auto px-4">
			{#if blocks.length > 0}
				<h3 class="px-2 pt-1 pb-1 text-xs font-medium text-muted-foreground">Panel</h3>
				<ul class="flex flex-col">
					{#each blocks as b (b.id)}
						{@render row(b.id, b.title, b.description, b.icon)}
					{/each}
				</ul>
			{/if}
			{#each groups as g (g.group)}
				<h3 class="px-2 pt-3 pb-1 text-xs font-medium text-muted-foreground">{g.group}</h3>
				<ul class="flex flex-col">
					{#each g.items as s (s.key)}
						{@render row(`setting:${s.key}`, s.label, s.description, s.icon)}
					{/each}
				</ul>
			{/each}
			{#if blocks.length === 0 && groups.length === 0}
				<p class="py-10 text-center text-sm text-subtle">Nothing matches “{query}”.</p>
			{/if}
		</div>
		<Dialog.Footer>
			<Button onclick={() => (open = false)}>Done</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
