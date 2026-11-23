<script lang="ts">
	import { onMount } from 'svelte';
	import type { AutoClipLogEntry, Cs2Config, GameEventKind, Settings } from '$lib/api/types';
	import * as ToggleGroup from '$lib/components/ui/toggle-group';
	import { Button } from '$lib/components/ui/button';
	import { Slider } from '$lib/components/ui/slider';
	import { Switch } from '$lib/components/ui/switch';
	import { api, errorMessage } from '$lib/api';
	import { app } from '$lib/stores/app.svelte';
	import NumberField from '../fields/NumberField.svelte';
	import SettingsRow from './SettingsRow.svelte';
	import { showAdvanced } from '$lib/stores/settingsNav.svelte';
	import SettingsSection from './SettingsSection.svelte';

	let { draft }: { draft: Settings } = $props();

	type RuleKey = 'kill' | 'headshot' | 'knife' | 'zeus' | 'collateral' | 'blindKill' | 'death' | 'roundWin';
	type Kind = { key: RuleKey; label: string; description: string };

	const THRESHOLDS = [
		{ kills: 1, label: 'Every kill' },
		{ kills: 2, label: '2K' },
		{ kills: 3, label: '3K' },
		{ kills: 4, label: '4K' },
		{ kills: 5, label: 'Ace' }
	];
	const SPECIAL: Kind[] = [
		{ key: 'headshot', label: 'Headshot', description: 'A one-kill clip with a headshot is named Headshot.' },
		{ key: 'knife', label: 'Knife kill', description: '' },
		{ key: 'zeus', label: 'Zeus kill', description: '' },
		{ key: 'collateral', label: 'Collateral', description: 'Two kills with one bullet.' },
		{ key: 'blindKill', label: 'Blind kill', description: 'A kill while you are flashed.' }
	];
	const OTHER: Kind[] = [
		{ key: 'death', label: 'Deaths', description: 'To see what happened.' },
		{ key: 'roundWin', label: 'Round wins', description: 'When your team wins the round.' }
	];
	const MODES = [
		{ id: 'competitive', label: 'Competitive and Premier' },
		{ id: 'wingman', label: 'Wingman' },
		{ id: 'casual', label: 'Casual' },
		{ id: 'deathmatch', label: 'Deathmatch' },
		{ id: 'armsrace', label: 'Arms Race' },
		{ id: 'other', label: 'Other modes' }
	];
	const KIND_NAMES: Record<GameEventKind, string> = {
		kill: 'Kill',
		headshot: 'Headshot',
		knife: 'Knife kill',
		zeus: 'Zeus kill',
		collateral: 'Collateral',
		blindKill: 'Blind kill',
		death: 'Death',
		roundWin: 'Round win',
		roundDone: 'Round over'
	};

	const auto = $derived(draft.autoClip);
	const volumePercent = $derived(Math.round(auto.soundVolume * 100));
	/** What the replay buffer holds: a round's clip can reach back no further. */
	const bufferSecs = $derived(draft.clip.lengthSecs + draft.clip.postSecs);

	// ---------- CS2 config file ----------

	/** The file holds the port; a changed one has to be applied before it is written. */
	const portPending = $derived(app.settings !== null && app.settings.autoClip.cs2.port !== auto.cs2.port);
	let installing = $state(false);
	let installNote = $state<{ ok: boolean; text: string } | null>(null);
	let manual = $state<Cs2Config | null>(null);
	let copied = $state(false);

	async function install() {
		installing = true;
		installNote = null;
		try {
			const path = await api.installCs2Config();
			installNote = { ok: true, text: `Written to ${path}. Restart CS2 if it is running.` };
			manual = null;
		} catch (error) {
			installNote = { ok: false, text: `${errorMessage(error)}. You can add the file by hand instead.` };
			await showFile();
		} finally {
			installing = false;
		}
	}

	async function showFile() {
		try {
			manual = await api.getCs2Config();
		} catch (error) {
			installNote = { ok: false, text: errorMessage(error) };
		}
	}

	async function copyFile() {
		if (!manual) return;
		await navigator.clipboard.writeText(manual.text);
		copied = true;
		setTimeout(() => (copied = false), 1500);
	}

	// ---------- Log ----------

	let log = $state<AutoClipLogEntry[]>([]);
	onMount(() => {
		let stop: (() => void) | undefined;
		let gone = false;
		void api.getAutoClipLog().then((entries) => {
			if (!gone) log = entries;
		});
		void api
			.onAutoClipLog((entry) => {
				log = [...log.slice(-199), entry];
			})
			.then((unlisten) => (gone ? unlisten() : (stop = unlisten)));
		return () => {
			gone = true;
			stop?.();
		};
	});
	const newestFirst = $derived([...log].reverse());

	function time(unixMs: number): string {
		const d = new Date(unixMs);
		return `${d.toLocaleTimeString(undefined, { hour12: false })}.${String(d.getMilliseconds()).padStart(3, '0')}`;
	}
</script>

{#snippet padding(key: RuleKey, label: string)}
	<div class="flex items-center gap-2" role="group" aria-labelledby="autoclip-{key}">
		<Switch bind:checked={draft.autoClip[key].enabled} aria-label="{label} makes a clip" />
		<NumberField
			class="flex-1"
			label="Seconds kept before"
			unit="s before"
			min={0}
			max={120}
			disabled={!draft.autoClip[key].enabled}
			bind:value={draft.autoClip[key].preSecs}
		/>
		<NumberField
			class="flex-1"
			label="Seconds kept after"
			unit="s after"
			min={0}
			max={60}
			disabled={!draft.autoClip[key].enabled}
			bind:value={draft.autoClip[key].postSecs}
		/>
	</div>
{/snippet}

<SettingsSection
	id="autoclip"
	title="Auto-clip"
	description="Saves a clip by itself when something happens in a game, named after what happened. Each game has its own section below."
>
	<SettingsRow
		label="Auto-clip"
		controlId="autoclip-on"
		description="Clips come from the replay buffer, with every audio track, like a clip saved with the hotkey. Umbra never reads the game's memory or touches its process: the game reports what happens itself."
	>
		<Switch id="autoclip-on" bind:checked={draft.autoClip.enabled} />
	</SettingsRow>

	{#if auto.enabled}
		<SettingsRow label="Sound" labelId="autoclip-volume" description="Played when an auto-clip is saved, at its own volume so it can be quieter than the hotkey's. 0% is silent.">
			<div class="flex items-center gap-3">
				<Slider
					type="single"
					min={0}
					max={100}
					step={5}
					value={volumePercent}
					onValueChange={(v) => (draft.autoClip.soundVolume = v / 100)}
					aria-labelledby="autoclip-volume"
				/>
				<span class="tnum w-10 shrink-0 text-right text-sm text-foreground">{volumePercent === 0 ? 'Off' : `${volumePercent}%`}</span>
			</div>
		</SettingsRow>
	{/if}
</SettingsSection>

{#if auto.enabled}
	<SettingsSection
		id="autoclip-cs2"
		page="autoclip"
		title="Counter-Strike 2"
		description="Uses CS2's Game State Integration, which Valve made for overlays and coaching tools: the game posts its state to Umbra on this PC (127.0.0.1) only. It needs a small config file in the game's folder."
	>
		<SettingsRow label="Clip Counter-Strike 2" controlId="autoclip-cs2-on">
			<Switch id="autoclip-cs2-on" bind:checked={draft.autoClip.cs2.enabled} />
			{#snippet below()}
				{#if auto.cs2.enabled}
					<div class="flex flex-col gap-2 rounded-md border border-line p-3">
						<p class="text-xs text-muted-foreground">
							Install the config file, then restart CS2. Install it again after updating Umbra: newer versions ask the game for more. The event log (shown with advanced settings on) says <span
								class="text-foreground">CS2 is connected</span
							> when the game reaches Umbra.
						</p>
						<div class="flex flex-wrap items-center gap-2">
							<Button size="sm" onclick={install} disabled={installing || portPending}>Install config file</Button>
							<Button size="sm" variant="ghost" onclick={showFile} disabled={portPending}>Show the file</Button>
							{#if portPending}<span class="text-xs text-muted-foreground">Apply the new port first.</span>{/if}
						</div>
						{#if installNote}
							<p class="selectable text-xs {installNote.ok ? 'text-foreground' : 'text-danger'}" role="status">{installNote.text}</p>
						{/if}
						{#if manual}
							<p class="text-xs text-muted-foreground">
								Save it as <span class="selectable text-foreground">{manual.fileName}</span> in
								{#if manual.folder}<span class="selectable text-foreground">{manual.folder}</span>{:else}the game's <span class="text-foreground">game\csgo\cfg</span> folder{/if}.
							</p>
							<pre class="selectable max-h-48 overflow-auto rounded bg-panel p-2 text-xs">{manual.text}</pre>
							<div><Button size="xs" variant="outline" onclick={copyFile}>{copied ? 'Copied' : 'Copy'}</Button></div>
						{/if}
					</div>
				{/if}
			{/snippet}
		</SettingsRow>

		{#if auto.cs2.enabled}
			<SettingsRow label="Game modes" labelId="autoclip-modes" description="Only these are clipped.">
				<ToggleGroup.Root
					type="multiple"
					variant="outline"
					spacing={1}
					class="grid w-full grid-cols-2"
					value={auto.cs2.modes}
					onValueChange={(next: string[]) => (draft.autoClip.cs2.modes = next)}
					aria-labelledby="autoclip-modes"
				>
					{#each MODES as mode (mode.id)}
						<ToggleGroup.Item value={mode.id}>{mode.label}</ToggleGroup.Item>
					{/each}
				</ToggleGroup.Root>
			</SettingsRow>

			<SettingsRow
				label="Kills"
				labelId="autoclip-kill"
				description="Counted per round: one clip covers the round's kills and is saved when the round ends, named Triple kill, Ace and so on. A round with fewer kills makes no clip."
			>
				<div class="flex flex-col gap-2">
					{@render padding('kill', 'Kills')}
					<ToggleGroup.Root
						type="single"
						variant="outline"
						spacing={1}
						class="grid w-full grid-cols-5"
						value={String(auto.minKills)}
						onValueChange={(next: string) => next && (draft.autoClip.minKills = Number(next))}
						disabled={!auto.kill.enabled}
						aria-label="Kills a clip needs"
					>
						{#each THRESHOLDS as t (t.kills)}
							<ToggleGroup.Item value={String(t.kills)} class="px-1">{t.label}</ToggleGroup.Item>
						{/each}
					</ToggleGroup.Root>
				</div>
				{#snippet below()}
					{#if auto.kill.enabled && bufferSecs < 120}
						<p class="text-xs text-muted-foreground">
							A round can last two minutes, and a clip reaches back only as far as the replay buffer: <span class="tnum text-foreground">{bufferSecs} s</span> now.
							The early kills of a long round are left out unless the clip length under Clip is 2 minutes or more.
						</p>
					{/if}
				{/snippet}
			</SettingsRow>

			<div class="border-t border-line pt-3" data-settings-row>
				<div class="text-sm font-medium text-foreground">Special kills</div>
				<p class="mt-1 text-xs text-muted-foreground">
					Always named in the clip, as in “Triple kill, Collateral”. Switched on, one makes a clip by itself, even in a round with fewer kills than set above.
				</p>
			</div>
			{#each SPECIAL as kind (kind.key)}
				<SettingsRow label={kind.label} labelId="autoclip-{kind.key}" description={kind.description || undefined}>
					{@render padding(kind.key, kind.label)}
				</SettingsRow>
			{/each}

			{#each OTHER as kind (kind.key)}
				<SettingsRow label={kind.label} labelId="autoclip-{kind.key}" description={kind.description}>
					{@render padding(kind.key, kind.label)}
				</SettingsRow>
			{/each}

			<SettingsRow advanced
				label="Merge window"
				controlId="autoclip-merge"
				description="Deathmatch and Arms Race have no rounds: there, kills this close together go in one clip, saved once this long passes without another."
			>
				<NumberField id="autoclip-merge" label="Merge window in seconds" unit="s" min={0} max={60} bind:value={draft.autoClip.mergeSecs} />
			</SettingsRow>

			<SettingsRow advanced label="Port" controlId="autoclip-port" description="Change it only if another program uses this port, then install the config file again.">
				<NumberField id="autoclip-port" label="Port" min={1024} max={65535} bind:value={draft.autoClip.cs2.port} />
			</SettingsRow>
		{/if}
	</SettingsSection>
{/if}

{#if showAdvanced()}
<SettingsSection
	id="autoclip-log"
	page="autoclip"
	title="Event log"
	description="What the detectors report and what was done about it, newest first. Also written to Umbra's log file."
>
	<div class="border-t border-line py-3" data-settings-row>
		{#if newestFirst.length === 0}
			<p class="text-xs text-subtle">Nothing yet.</p>
		{:else}
			<div class="max-h-72 overflow-auto rounded-md border border-line">
				<table class="w-full text-xs" aria-labelledby="autoclip-log-title">
					<thead class="sticky top-0 bg-panel text-left text-muted-foreground">
						<tr>
							<th class="px-2 py-1 font-medium">Time</th>
							<th class="px-2 py-1 font-medium">Event</th>
							<th class="px-2 py-1 text-right font-medium">Confidence</th>
							<th class="px-2 py-1 font-medium">Source</th>
							<th class="px-2 py-1 font-medium">What happened</th>
						</tr>
					</thead>
					<tbody>
						{#each newestFirst as entry, i (i)}
							<tr class="border-t border-line align-top">
								<td class="tnum px-2 py-1 whitespace-nowrap text-muted-foreground">{time(entry.unixMs)}</td>
								<td class="px-2 py-1 whitespace-nowrap">{entry.kind ? KIND_NAMES[entry.kind] : ''}</td>
								<td class="tnum px-2 py-1 text-right">{entry.confidence === null ? '' : entry.confidence.toFixed(2)}</td>
								<td class="px-2 py-1 whitespace-nowrap text-muted-foreground">{entry.source ?? ''}</td>
								<td class="selectable px-2 py-1">{entry.message}</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		{/if}
	</div>
</SettingsSection>
{/if}
