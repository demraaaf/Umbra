<script lang="ts">
	import { flip } from 'svelte/animate';
	import MinusIcon from '@lucide/svelte/icons/minus';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import SlidersHorizontalIcon from '@lucide/svelte/icons/sliders-horizontal';
	import { Button } from '$lib/components/ui/button';
	import { app, patchSettings } from '$lib/stores/app.svelte';
	import { DEFAULT_PANEL, isHalf, widgetFor } from '$lib/panel/widgets';
	import CaptureActions from './CaptureActions.svelte';
	import MicControl from './MicControl.svelte';
	import CameraControl from './CameraControl.svelte';
	import InputControl from './InputControl.svelte';
	import SettingWidget from './SettingWidget.svelte';
	import SourceSwitcher from './SourceSwitcher.svelte';
	import StatusBlock from './StatusBlock.svelte';
	import UsageMeters from './UsageMeters.svelte';
	import WidgetPicker from './WidgetPicker.svelte';

	/**
	 * The recording panel: widgets you choose and order, like a control center. Customize
	 * turns on editing: drag a widget to move it (or focus it and use the arrow keys), remove
	 * it, add the panel's parts or any setting. Nothing changes until Save; it is kept in the
	 * settings from then on.
	 */

	let editing = $state(false);
	let draft = $state<string[]>([]);
	let picking = $state(false);
	let saving = $state(false);

	const saved = $derived(app.settings?.panel?.widgets ?? DEFAULT_PANEL);
	const ids = $derived((editing ? draft : saved).filter((id) => widgetFor(id)));

	/** Each widget with its width, and for half-width ones, which side of the row it is on. */
	const layout = $derived.by(() => {
		let column = 0;
		return ids.map((id) => {
			const half = isHalf(id);
			const side = half ? (column === 0 ? 'left' : 'right') : 'full';
			column = half ? (column + 1) % 2 : 0;
			return { id, half, side };
		});
	});

	function edit() {
		draft = [...saved];
		editing = true;
	}

	function cancel() {
		editing = false;
		dragging = null;
	}

	async function save() {
		saving = true;
		const widgets = [...draft];
		const ok = await patchSettings((s) => {
			s.panel = { widgets };
		});
		saving = false;
		if (ok) editing = false;
	}

	function remove(id: string) {
		draft = draft.filter((d) => d !== id);
	}

	function add(id: string) {
		if (!draft.includes(id)) draft = [...draft, id];
	}

	function move(id: string, to: number) {
		const from = draft.indexOf(id);
		if (from < 0 || to < 0 || to >= draft.length || from === to) return;
		const next = [...draft];
		next.splice(from, 1);
		next.splice(to, 0, id);
		draft = next;
	}

	// ---------- Dragging ----------

	let dragging = $state<string | null>(null);

	function grab(event: PointerEvent, id: string) {
		if (!editing || event.button !== 0) return;
		if ((event.target as HTMLElement).closest('.remove')) return;
		event.preventDefault();
		dragging = id;
		(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
	}

	function drag(event: PointerEvent) {
		if (!dragging) return;
		// The widget under the pointer takes the dragged one's place.
		const over = document.elementFromPoint(event.clientX, event.clientY)?.closest<HTMLElement>('[data-widget]');
		const target = over?.dataset.widget;
		if (target && target !== dragging) move(dragging, draft.indexOf(target));
	}

	function drop() {
		dragging = null;
	}

	function keys(event: KeyboardEvent, id: string) {
		if (!editing) return;
		const at = draft.indexOf(id);
		if (event.key === 'ArrowUp' || event.key === 'ArrowLeft') move(id, at - 1);
		else if (event.key === 'ArrowDown' || event.key === 'ArrowRight') move(id, at + 1);
		else if (event.key === 'Delete' || event.key === 'Backspace') remove(id);
		else return;
		event.preventDefault();
		// Keep the focus on the moved widget.
		requestAnimationFrame(() => document.querySelector<HTMLElement>(`[data-widget="${CSS.escape(id)}"]`)?.focus());
	}
</script>

<!-- The recording panel: everything about the live recorder, arranged by you. -->
<aside class="flex w-(--panel-w) shrink-0 flex-col overflow-y-auto border-l border-line bg-chrome" aria-label="Recording panel">
	{#if editing}
		<div class="sticky top-0 z-10 flex flex-col gap-2 border-b border-line bg-chrome px-3 py-3">
			<div class="flex items-center gap-2">
				<span class="flex-1 text-sm font-medium text-foreground">Customize the panel</span>
				<Button variant="ghost" size="xs" onclick={() => (draft = [...DEFAULT_PANEL])} title="Back to how Umbra came">Reset</Button>
			</div>
			<p class="text-xs text-subtle">Drag to move, or focus one and use the arrow keys.</p>
			<div class="flex gap-2">
				<Button variant="outline" size="sm" class="flex-1" onclick={cancel} disabled={saving}>Cancel</Button>
				<Button size="sm" class="flex-1" onclick={save} disabled={saving}>Save</Button>
			</div>
		</div>
	{/if}

	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="grid grid-cols-2" class:editing onpointermove={drag} onpointerup={drop} onpointercancel={drop}>
		{#each layout as item (item.id)}
			{@const def = widgetFor(item.id)}
			<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
			<div
				class="widget relative min-w-0 {item.side}"
				class:col-span-2={!item.half}
				class:dragging={dragging === item.id}
				data-widget={item.id}
				role={editing ? 'listitem' : undefined}
				aria-label={editing ? `${def?.kind === 'block' ? def.title : def?.label}, press arrow keys to move, Delete to remove` : undefined}
				tabindex={editing ? 0 : undefined}
				onpointerdown={(e) => grab(e, item.id)}
				onkeydown={(e) => keys(e, item.id)}
				animate:flip={{ duration: 180 }}
			>
				<div class="content" inert={editing}>
					{#if def?.kind === 'block'}
						{#if def.id === 'status'}<StatusBlock />
						{:else if def.id === 'actions'}<CaptureActions />
						{:else if def.id === 'source'}<SourceSwitcher />
						{:else if def.id === 'mic'}<MicControl />
						{:else if def.id === 'camera'}<CameraControl />
						{:else if def.id === 'input'}<InputControl />
						{:else if def.id === 'usage'}<UsageMeters />{/if}
					{:else if def}
						<SettingWidget {def} />
					{/if}
				</div>
				{#if editing}
					<button
						type="button"
						class="remove"
						onclick={() => remove(item.id)}
						aria-label="Remove {def?.kind === 'block' ? def.title : def?.label}"
						title="Remove"
					>
						<MinusIcon class="size-3" />
					</button>
				{/if}
			</div>
		{/each}
	</div>

	{#if editing}
		<div class="p-3">
			<Button variant="outline" size="sm" class="w-full" onclick={() => (picking = true)}>
				<PlusIcon data-icon="inline-start" />
				Add widget
			</Button>
		</div>
	{:else}
		<div class="mt-auto flex justify-center p-2">
			<Button variant="ghost" size="xs" class="text-subtle" onclick={edit} title="Choose what the panel shows and in which order">
				<SlidersHorizontalIcon data-icon="inline-start" />
				Customize
			</Button>
		</div>
	{/if}
</aside>

<WidgetPicker bind:open={picking} added={draft} onadd={add} />

<style>
	/* Half-width tiles sit two to a row, padded like the full-width sections around them. */
	.widget.left {
		padding: 6px 3px 6px 12px;
	}
	.widget.right {
		padding: 6px 12px 6px 3px;
	}
	.editing .widget {
		cursor: grab;
		touch-action: none;
		outline: 1px dashed var(--line-strong);
		outline-offset: -4px;
		border-radius: 8px;
	}
	.editing .widget:focus-visible {
		outline: 2px solid var(--focus);
	}
	.editing .widget.dragging {
		cursor: grabbing;
		z-index: 1;
		background: var(--surface-raised);
		box-shadow: 0 8px 24px rgb(0 0 0 / 0.35);
	}
	.editing .content {
		user-select: none;
	}
	.remove {
		position: absolute;
		top: 4px;
		right: 4px;
		display: flex;
		width: 20px;
		height: 20px;
		align-items: center;
		justify-content: center;
		border-radius: 999px;
		background: var(--text);
		color: var(--background);
		cursor: pointer;
		transition: transform var(--dur-fast) var(--ease-out);
	}
	.remove:active {
		transform: scale(0.92);
	}
	.remove:focus-visible {
		outline: 2px solid var(--focus);
		outline-offset: 2px;
	}
</style>
