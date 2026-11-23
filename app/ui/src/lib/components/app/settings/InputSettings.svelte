<script lang="ts">
	import { onMount } from 'svelte';
	import type { InputRule, KeyboardMode, MouseMode, Settings } from '$lib/api/types';
	import * as ToggleGroup from '$lib/components/ui/toggle-group';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { Slider } from '$lib/components/ui/slider';
	import { Switch } from '$lib/components/ui/switch';
	import { app } from '$lib/stores/app.svelte';
	import { keyLabel } from '$lib/format/hotkey';
	import { DEFAULT_INPUT_RULES } from '$lib/input/overlay';
	import ArrowUpIcon from '@lucide/svelte/icons/arrow-up';
	import ArrowDownIcon from '@lucide/svelte/icons/arrow-down';
	import GripVerticalIcon from '@lucide/svelte/icons/grip-vertical';
	import XIcon from '@lucide/svelte/icons/x';
	import { api } from '$lib/api';
	import ExeList from './ExeList.svelte';
	import HotkeyRecorder from './HotkeyRecorder.svelte';
	import SettingsRow from './SettingsRow.svelte';
	import { showAdvanced } from '$lib/stores/settingsNav.svelte';
	import SettingsSection from './SettingsSection.svelte';

	let { draft }: { draft: Settings } = $props();

	const input = $derived(draft.input);
	const live = $derived(app.status?.input);

	const MATCHES: { value: InputRule['matches']; label: string }[] = [
		{ value: 'game', label: 'Detected game' },
		{ value: 'apps', label: 'Programs' },
		{ value: 'any', label: 'Every program' }
	];
	const KEYBOARD: { value: KeyboardMode; label: string }[] = [
		{ value: 'full', label: 'Every key' },
		{ value: 'redacted', label: 'Hide typing' },
		{ value: 'drop', label: 'Nothing' }
	];
	const MOUSE: { value: MouseMode; label: string }[] = [
		{ value: 'full', label: 'Everything' },
		{ value: 'buttonsOnly', label: 'Buttons' },
		{ value: 'drop', label: 'Nothing' }
	];

	// ---------- Rules ----------

	function move(from: number, to: number) {
		const rules = draft.input.rules;
		if (to < 0 || to >= rules.length || from === to) return;
		const [rule] = rules.splice(from, 1);
		rules.splice(to, 0, rule);
	}

	function addRule() {
		// Above the catch-all at the end, so it can apply at all.
		const at = draft.input.rules.findIndex((r) => r.matches === 'any');
		const rule: InputRule = { name: 'New rule', matches: 'apps', apps: [], keyboard: 'redacted', mouse: 'full', chatKeys: [] };
		draft.input.rules.splice(at < 0 ? draft.input.rules.length : at, 0, rule);
	}

	/** Rules open for editing; the others show a line saying what they do. */
	let open = $state<InputRule[]>([]);
	const toggle = (rule: InputRule) =>
		(open = open.includes(rule) ? open.filter((r) => r !== rule) : [...open, rule]);
	const label = <T extends { value: string; label: string }>(list: T[], value: string) =>
		list.find((m) => m.value === value)?.label ?? value;
	function summary(rule: InputRule): string {
		const where =
			rule.matches === 'game'
				? 'The detected game'
				: rule.matches === 'any'
					? 'Every program'
					: rule.apps.length === 1
						? rule.apps[0]
						: `${rule.apps.length} programs`;
		return `${where} · keyboard: ${label(KEYBOARD, rule.keyboard).toLowerCase()} · mouse: ${label(MOUSE, rule.mouse).toLowerCase()}`;
	}

	/** Programs with a window now, offered for a rule. */
	let running = $state<{ exe: string; name: string; group: string }[]>([]);
	onMount(() => {
		void api
			.listWindows()
			.then((windows) => {
				const seen = new Set<string>();
				running = windows
					.filter((w) => !seen.has(w.exe.toLowerCase()) && seen.add(w.exe.toLowerCase()))
					.map((w) => ({ exe: w.exe, name: w.title, group: 'Running now' }));
			})
			.catch(() => {});
	});

	let dragging = $state<number | null>(null);
	let dropAt = $state<number | null>(null);

	/** The rule whose chat key is being recorded. */
	let recording = $state<number | null>(null);
	$effect(() => {
		const index = recording;
		if (index === null) return;
		const onKey = (e: KeyboardEvent) => {
			e.preventDefault();
			e.stopPropagation();
			recording = null;
			if (e.code === 'Escape' && !e.shiftKey) return;
			const keys = draft.input.rules[index]?.chatKeys;
			if (keys && !keys.includes(e.code)) keys.push(e.code);
		};
		window.addEventListener('keydown', onKey, true);
		return () => window.removeEventListener('keydown', onKey, true);
	});

	const opacityPercent = $derived(Math.round(input.overlay.opacity * 100));
</script>

<SettingsSection
	id="input"
	page="overlay"
	title="Keyboard and mouse"
	description="Shows the keys and mouse buttons you press over your clips."
>
	<SettingsRow
		label="Record keyboard and mouse"
		controlId="input-on"
		description="Saved in a small file next to each clip, not in the video, so a clip you share has none of it unless you export it with the overlay. Nothing is recorded from password fields or while Umbra is in front. Umbra reads input the way any program can and does not hook into games."
	>
		<Switch id="input-on" bind:checked={draft.input.enabled} />
		{#snippet below()}
			{#if live?.on}
				<p class="text-xs text-muted-foreground" role="status" aria-live="polite">
					Now: <span class="text-foreground">{live.app ?? 'nothing in front'}</span>
					→ {#if live.rule}<span class="text-foreground">{live.rule}</span>{:else}<span class="text-foreground">nothing kept</span>{/if}
					{#if live.guard === 'password'}· <span class="text-foreground">password field, no keys kept</span>{/if}
					{#if live.guard === 'chat'}· <span class="text-foreground">chat open, typing hidden</span>{/if}
				</p>
			{/if}
		{/snippet}
	</SettingsRow>

	<SettingsRow label="Hotkey" labelId="input-hotkey" description="Switches recording keyboard and mouse on or off. None by default.">
		<HotkeyRecorder bind:value={draft.input.hotkey} labelledBy="input-hotkey" />
	</SettingsRow>

	{#if showAdvanced()}
	<div class="border-t border-line py-3" data-settings-row>
		<div class="flex items-center justify-between gap-2">
			<div class="text-sm font-medium text-foreground" id="input-rules">What is kept, per program</div>
			<div class="flex gap-1">
				<Button size="xs" variant="ghost" onclick={() => (draft.input.rules = structuredClone(DEFAULT_INPUT_RULES))}>Reset</Button>
				<Button size="xs" variant="outline" onclick={addRule}>Add rule</Button>
			</div>
		</div>
		<p class="mt-1 text-xs text-muted-foreground">
			Decides what is recorded in the program in front. The first rule that matches wins; drag to reorder. A program no rule
			matches records nothing. “Hide typing” records Shift, the arrows, F-keys and shortcuts, and shows any other key as a dot.
		</p>
		<ol class="mt-3 flex flex-col gap-2" aria-labelledby="input-rules">
			{#each input.rules as rule, i (rule)}
				<li
					class="rounded-md border p-3 {dropAt === i && dragging !== i ? 'border-foreground' : 'border-line'} {dragging === i ? 'opacity-50' : ''}"
					ondragover={(e) => {
						if (dragging === null) return;
						e.preventDefault();
						dropAt = i;
					}}
					ondrop={(e) => {
						e.preventDefault();
						if (dragging !== null) move(dragging, i);
						dragging = null;
						dropAt = null;
					}}
				>
					<div class="flex items-center gap-2">
						<span
							class="cursor-grab text-subtle"
							draggable="true"
							role="button"
							tabindex="-1"
							aria-label="Drag to reorder"
							ondragstart={(e) => {
								dragging = i;
								e.dataTransfer?.setData('text/plain', String(i));
							}}
							ondragend={() => {
								dragging = null;
								dropAt = null;
							}}
						>
							<GripVerticalIcon class="size-4" />
						</span>
						<span class="tnum w-4 text-xs text-subtle">{i + 1}</span>
						{#if open.includes(rule)}
							<Input class="h-7 min-w-0 flex-1 text-sm" bind:value={rule.name} aria-label="Rule name" />
						{:else}
							<button type="button" class="min-w-0 flex-1 truncate text-left" onclick={() => toggle(rule)} aria-expanded="false">
								<span class="text-sm text-foreground">{rule.name}</span>
								<span class="ml-2 text-xs text-muted-foreground">{summary(rule)}</span>
							</button>
						{/if}
						<Button size="xs" variant="ghost" onclick={() => toggle(rule)} aria-expanded={open.includes(rule)}>
							{open.includes(rule) ? 'Done' : 'Edit'}
						</Button>
						<Button size="icon-xs" variant="ghost" onclick={() => move(i, i - 1)} disabled={i === 0} aria-label="Move up">
							<ArrowUpIcon />
						</Button>
						<Button size="icon-xs" variant="ghost" onclick={() => move(i, i + 1)} disabled={i === input.rules.length - 1} aria-label="Move down">
							<ArrowDownIcon />
						</Button>
						<Button size="icon-xs" variant="ghost" onclick={() => draft.input.rules.splice(i, 1)} aria-label="Remove rule">
							<XIcon />
						</Button>
					</div>
					{#if open.includes(rule)}
					<div class="mt-3 grid grid-cols-[6rem_minmax(0,1fr)] items-center gap-x-3 gap-y-2 text-xs">
						<span class="text-muted-foreground">Applies to</span>
						<ToggleGroup.Root
							type="single"
							variant="outline"
							size="sm"
							class="grid w-full grid-cols-3"
							value={rule.matches}
							onValueChange={(v: string) => v && (rule.matches = v as InputRule['matches'])}
							aria-label="Applies to"
						>
							{#each MATCHES as m (m.value)}<ToggleGroup.Item value={m.value} class="px-1 text-xs">{m.label}</ToggleGroup.Item>{/each}
						</ToggleGroup.Root>
						{#if rule.matches === 'apps'}
							<span class="text-muted-foreground">Programs</span>
							<ExeList
								bind:items={rule.apps}
								inputId="input-rule-{i}"
								suggestions={running}
								suggestLabel="Running now"
								paths
								folders
								emptyText="No program yet."
							/>
						{/if}
						<span class="text-muted-foreground">Keyboard</span>
						<ToggleGroup.Root
							type="single"
							variant="outline"
							size="sm"
							class="grid w-full grid-cols-3"
							value={rule.keyboard}
							onValueChange={(v: string) => v && (rule.keyboard = v as KeyboardMode)}
							aria-label="Keyboard"
						>
							{#each KEYBOARD as m (m.value)}<ToggleGroup.Item value={m.value} class="px-1 text-xs">{m.label}</ToggleGroup.Item>{/each}
						</ToggleGroup.Root>
						<span class="text-muted-foreground">Mouse</span>
						<ToggleGroup.Root
							type="single"
							variant="outline"
							size="sm"
							class="grid w-full grid-cols-3"
							value={rule.mouse}
							onValueChange={(v: string) => v && (rule.mouse = v as MouseMode)}
							aria-label="Mouse"
						>
							{#each MOUSE as m (m.value)}<ToggleGroup.Item value={m.value} class="px-1 text-xs">{m.label}</ToggleGroup.Item>{/each}
						</ToggleGroup.Root>
						{#if rule.keyboard === 'full'}
							<span class="text-muted-foreground" title="Typing is hidden from one of these keys until Enter or Esc">Chat keys</span>
							<div class="flex flex-wrap items-center gap-1">
								{#each rule.chatKeys as key, k (key)}
									<span class="inline-flex items-center gap-1 rounded border border-line px-1.5 py-0.5">
										{keyLabel(key)}
										<button type="button" class="text-subtle hover:text-foreground" aria-label="Remove {keyLabel(key)}" onclick={() => rule.chatKeys.splice(k, 1)}>
											<XIcon class="size-3" />
										</button>
									</span>
								{/each}
								<Button size="xs" variant="ghost" onclick={() => (recording = recording === i ? null : i)}>
									{recording === i ? 'Press a key…' : 'Add key'}
								</Button>
							</div>
						{/if}
					</div>
					{/if}
				</li>
			{/each}
		</ol>
	</div>
	{/if}

	<SettingsRow label="Show" labelId="input-keys" description="What new clips show. Place it under Position at the top of this page; the editor can change it per clip.">
		<div class="flex flex-col gap-2">
			<ToggleGroup.Root
				type="single"
				variant="outline"
				class="grid w-full grid-cols-3"
				value={input.overlay.keys}
				onValueChange={(v: string) => v && (draft.input.overlay.keys = v as Settings['input']['overlay']['keys'])}
				aria-labelledby="input-keys"
			>
				<ToggleGroup.Item value="wasd">WASD</ToggleGroup.Item>
				<ToggleGroup.Item value="keyboard">Keyboard</ToggleGroup.Item>
				<ToggleGroup.Item value="none">No keys</ToggleGroup.Item>
			</ToggleGroup.Root>
			<label class="flex items-center justify-between gap-2 text-sm">Mouse <Switch bind:checked={draft.input.overlay.mouse} /></label>
			<label class="flex items-center justify-between gap-2 text-sm">
				Mouse movement <Switch bind:checked={draft.input.overlay.movement} disabled={!input.overlay.mouse} />
			</label>
			<label class="flex items-center justify-between gap-2 text-sm">Keys pressed last <Switch bind:checked={draft.input.overlay.ticker} /></label>
		</div>
	</SettingsRow>

	<SettingsRow label="Opacity" labelId="input-opacity">
		<div class="flex items-center gap-3">
			<Slider
				type="single"
				min={10}
				max={100}
				step={5}
				value={opacityPercent}
				onValueChange={(v) => (draft.input.overlay.opacity = v / 100)}
				aria-labelledby="input-opacity"
			/>
			<span class="tnum w-10 shrink-0 text-right text-sm text-foreground">{opacityPercent}%</span>
		</div>
	</SettingsRow>

	<SettingsRow advanced label="Held keys" controlId="input-highlight" description="The colour of keys and buttons while they are down.">
		<input id="input-highlight" type="color" class="h-8 w-14 cursor-pointer rounded border border-line bg-transparent" bind:value={draft.input.overlay.highlight} />
	</SettingsRow>
</SettingsSection>
