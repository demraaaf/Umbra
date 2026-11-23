<script lang="ts">
	import { onMount, tick, untrack } from 'svelte';
	import { layout } from '$lib/stores/layout.svelte';
	import { SETTINGS_GROUPS, openSettingsPage, setAdvanced, settingsNav } from '$lib/stores/settingsNav.svelte';
	import { Switch } from '$lib/components/ui/switch';
	import SearchIcon from '@lucide/svelte/icons/search';
	import * as InputGroup from '$lib/components/ui/input-group';
	import CheckIcon from '@lucide/svelte/icons/check';
	import { errorMessage } from '$lib/api';
	import { Button } from '$lib/components/ui/button';
	import { Spinner } from '$lib/components/ui/spinner';
	import AboutSettings from '$lib/components/app/settings/AboutSettings.svelte';
	import HelpSettings from '$lib/components/app/settings/HelpSettings.svelte';
	import PlayerSettings from '$lib/components/app/settings/PlayerSettings.svelte';
	import ScreenshotSettings from '$lib/components/app/settings/ScreenshotSettings.svelte';
	import UsageEstimate from '$lib/components/app/settings/UsageEstimate.svelte';
	import AudioSettings from '$lib/components/app/settings/AudioSettings.svelte';
	import OverlaySettings from '$lib/components/app/settings/OverlaySettings.svelte';
	import InputSettings from '$lib/components/app/settings/InputSettings.svelte';
	import CaptureSettings from '$lib/components/app/settings/CaptureSettings.svelte';
	import ClipSettings from '$lib/components/app/settings/ClipSettings.svelte';
	import AutoClipSettings from '$lib/components/app/settings/AutoClipSettings.svelte';
	import RecordingSettings from '$lib/components/app/settings/RecordingSettings.svelte';
	import StorageSettings from '$lib/components/app/settings/StorageSettings.svelte';
	import VideoSettings from '$lib/components/app/settings/VideoSettings.svelte';
	import { app, applyDraft, draftIsDirty, resetDraft } from '$lib/stores/app.svelte';

	/** Pages whose settings change what recording costs, so the estimate shows on them. */
	const COSTLY = new Set(['capture', 'video', 'audio', 'overlay', 'clip', 'recording']);
	let scroller = $state<HTMLDivElement | null>(null);
	const active = $derived(settingsNav.page);
	let applying = $state(false);
	let applied = $state(false);
	let applyError = $state<string | null>(null);
	let appliedTimer: ReturnType<typeof setTimeout> | null = null;

	const dirty = $derived(draftIsDirty());

	$effect(() => {
		if (dirty) {
			applied = false;
			applyError = null;
		}
	});

	async function apply() {
		if (!dirty || applying) return;
		applying = true;
		applyError = null;
		try {
			await applyDraft();
			applied = true;
			if (appliedTimer) clearTimeout(appliedTimer);
			appliedTimer = setTimeout(() => (applied = false), 3000);
		} catch (error) {
			applyError = errorMessage(error);
		} finally {
			applying = false;
		}
	}

	function discard() {
		resetDraft();
		applyError = null;
	}

	// ---------- Search ----------
	// Rows whose label, description or controls mention every typed word stay; a section whose
	// title matches keeps all its rows. Ctrl+F jumps here.

	let matches = $state(0);
	const query = $derived(settingsNav.query);
	const searching = $derived(query.trim() !== '');
	/** A page is drawn when it is open, or while searching (every page, to find matches). */
	const shows = (id: string) => searching || active === id;
	let searchInput = $state<HTMLInputElement | null>(null);

	function filter(text: string) {
		if (!scroller) return;
		const words = text.toLowerCase().split(/\s+/).filter(Boolean);
		const has = (el: Element) => {
			const content = (el.textContent ?? '').toLowerCase();
			return words.every((w) => content.includes(w));
		};
		let count = 0;
		const perPage: Record<string, number> = {};
		for (const section of scroller.querySelectorAll<HTMLElement>('[data-section]')) {
			if (words.length === 0) {
				section.hidden = false;
				section.querySelectorAll<HTMLElement>('[data-settings-row]').forEach((row) => (row.hidden = false));
				continue;
			}
			const heading = section.querySelector('h2');
			const whole = heading ? has(heading) : false;
			const rows = [...section.querySelectorAll<HTMLElement>('[data-settings-row]')];
			let shown = 0;
			for (const row of rows) {
				row.hidden = !(whole || has(row));
				if (!row.hidden) shown++;
			}
			// Sections without rows (Help, About) match on their whole text.
			const visible = rows.length > 0 ? shown > 0 : whole || has(section);
			section.hidden = !visible;
			if (visible) count += Math.max(shown, 1);
			const page = section.dataset.section ?? '';
			perPage[page] = (perPage[page] ?? 0) + (visible ? Math.max(shown, 1) : 0);
		}
		matches = count;
		settingsNav.counts = perPage;
	}

	$effect(() => {
		const text = query;
		void tick().then(() => {
			filter(text);
			if (text.trim()) scroller?.scrollTo({ top: 0 });
		});
	});

	function jump(id: string) {
		openSettingsPage(id);
		scroller?.scrollTo({ top: 0 });
	}

	// Opening a page from the sidebar starts it at the top.
	$effect(() => {
		void settingsNav.page;
		untrack(() => scroller?.scrollTo({ top: 0 }));
	});

	onMount(() => {
		const onKey = (e: KeyboardEvent) => {
			if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') {
				e.preventDefault();
				void apply();
			}
			if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'f') {
				e.preventDefault();
				searchInput?.focus();
				searchInput?.select();
			}
		};
		window.addEventListener('keydown', onKey);

		return () => {
			window.removeEventListener('keydown', onKey);
			if (appliedTimer) clearTimeout(appliedTimer);
		};
	});
</script>

<section class="flex min-h-0 flex-1 flex-col" aria-label="Settings">
	<div class="flex min-h-0 flex-1">
		{#if layout.leftCollapsed}
		<nav class="pages flex w-48 shrink-0 flex-col gap-4 overflow-y-auto border-r border-line px-3 py-3" aria-label="Settings pages">
			{#each SETTINGS_GROUPS as group (group.label)}
				<div class="flex flex-col gap-0.5" role="group" aria-label={group.label}>
					<span class="px-2 pb-1 text-xs text-subtle">{group.label}</span>
					{#each group.pages as page (page.id)}
						{@const found = settingsNav.counts[page.id] ?? 0}
						<button
							type="button"
							class="page-link"
							class:active={!searching && active === page.id}
							class:empty={searching && found === 0}
							aria-current={!searching && active === page.id ? 'page' : undefined}
							onclick={() => jump(page.id)}
						>
							<span class="truncate">{page.label}</span>
							{#if searching && found > 0}
								<span class="tnum ml-auto text-xs text-muted-foreground" aria-label="{found} matching">{found}</span>
							{/if}
						</button>
					{/each}
				</div>
			{/each}
		</nav>
		{/if}

		<div class="flex min-h-0 min-w-0 flex-1 flex-col">
		<div class="flex h-12 shrink-0 items-center border-b border-line px-6">
			<InputGroup.Root class="w-72">
				<InputGroup.Addon><SearchIcon aria-hidden="true" /></InputGroup.Addon>
				<InputGroup.Input
					bind:ref={searchInput}
					bind:value={settingsNav.query}
					type="search"
					placeholder="Search all settings"
					aria-label="Search settings"
					autocomplete="off"
					class="[&::-webkit-search-cancel-button]:hidden"
					onkeydown={(e) => {
						if (e.key === 'Escape' && settingsNav.query) {
							e.preventDefault();
							settingsNav.query = '';
						}
					}}
				/>
			</InputGroup.Root>
			{#if searching}
				<span class="tnum ml-3 text-xs text-muted-foreground" role="status">
					{matches === 0 ? 'No matches' : matches === 1 ? '1 match' : `${matches} matches`}
				</span>
			{/if}
			<div class="ml-auto shrink-0">
				<label
					class="advanced-switch flex items-center gap-2 rounded-md px-2 py-1 text-sm text-muted-foreground"
					class:hint={settingsNav.advancedHint && !settingsNav.advanced}
					for="settings-advanced"
				>
					Show advanced settings
					<Switch
						id="settings-advanced"
						checked={settingsNav.advanced}
						onCheckedChange={setAdvanced}
						aria-describedby={settingsNav.advancedHint && !settingsNav.advanced ? 'advanced-callout' : undefined}
					/>
				</label>
			</div>
		</div>
		<!-- In the flow, under the switch, so it pushes the page down instead of covering it. -->
		{#if settingsNav.advancedHint && !settingsNav.advanced}
			<div id="advanced-callout" class="callout relative shrink-0 border-b border-line bg-surface-raised px-6 py-2 text-right text-xs text-muted-foreground">
				<span class="font-medium text-foreground">Know your way around encoders?</span>
				Advanced settings open up every recording option, from codecs to bitrates.
			</div>
		{/if}
		<div class="min-h-0 min-w-0 flex-1 overflow-y-auto" bind:this={scroller}>
			{#if app.draft}
				<div class="@container mx-auto w-full max-w-[820px] px-6 pb-16">
					{#if !searching && COSTLY.has(active)}<UsageEstimate draft={app.draft} />{/if}
					{#if shows('capture')}<CaptureSettings draft={app.draft} />{/if}
					{#if shows('video')}<VideoSettings draft={app.draft} />{/if}
					{#if shows('audio')}<AudioSettings draft={app.draft} />{/if}
					{#if shows('overlay')}<OverlaySettings draft={app.draft} /><InputSettings draft={app.draft} />{/if}
					{#if shows('clip')}<ClipSettings draft={app.draft} />{/if}
					{#if shows('autoclip')}<AutoClipSettings draft={app.draft} />{/if}
					{#if shows('screenshots')}<ScreenshotSettings draft={app.draft} />{/if}
					{#if shows('recording')}<RecordingSettings draft={app.draft} />{/if}
					{#if shows('player')}<PlayerSettings draft={app.draft} />{/if}
					{#if shows('storage')}<StorageSettings draft={app.draft} />{/if}
					{#if shows('help') && app.settings}<HelpSettings settings={app.settings} />{/if}
					{#if shows('about')}<AboutSettings />{/if}
					{#if searching && matches === 0}
						<p class="py-16 text-center text-sm text-muted-foreground" role="status">No setting matches “{query.trim()}”.</p>
					{/if}
				</div>
			{/if}
		</div>
		</div>
	</div>

	<div class="flex h-12 shrink-0 items-center gap-3 border-t border-line bg-panel px-4" role="region" aria-label="Apply settings">
		<p class="flex min-w-0 flex-1 items-center gap-2 truncate text-sm" role="status" aria-live="polite">
			{#if applyError}
				<span class="selectable truncate text-danger">Could not apply: {applyError}</span>
			{:else if applied && !dirty}
				<CheckIcon class="size-4 shrink-0 text-success" strokeWidth={2.5} aria-hidden="true" />
				<span class="text-foreground">Applied</span>
			{:else if dirty}
				<span class="text-foreground">You have unapplied changes.</span>
			{:else}
				<span class="text-muted-foreground">All changes are applied.</span>
			{/if}
		</p>
		<Button variant="destructive" onclick={discard} disabled={!dirty || applying} title="Throw away the changes you have not applied">Discard</Button>
		<Button onclick={apply} disabled={!dirty || applying} title="Apply (Ctrl+S)">
			{#if applying}<Spinner data-icon="inline-start" />{/if}
			Apply
		</Button>
	</div>
</section>

<style>
	/* Points new users at the switch until they have tried it once: a steady hairline, and
	   two soft rings that spread out and fade, half a beat apart. */
	.advanced-switch {
		position: relative;
	}
	.advanced-switch.hint {
		box-shadow: 0 0 0 1px color-mix(in oklab, var(--focus) 35%, transparent);
	}
	.advanced-switch.hint::before,
	.advanced-switch.hint::after {
		content: '';
		position: absolute;
		inset: 0;
		border-radius: inherit;
		pointer-events: none;
		animation: ring 2.4s cubic-bezier(0.16, 1, 0.3, 1) infinite;
	}
	.advanced-switch.hint::after {
		animation-delay: 1.2s;
	}
	@keyframes ring {
		0% {
			box-shadow: 0 0 0 0 color-mix(in oklab, var(--focus) 45%, transparent);
		}
		100% {
			box-shadow: 0 0 0 10px color-mix(in oklab, var(--focus) 0%, transparent);
		}
	}
	.callout {
		animation: callout-in 260ms var(--ease-out) 300ms both;
	}
	/* A small arrow up to the switch. */
	.callout::before {
		content: '';
		position: absolute;
		top: -5px;
		right: 46px;
		width: 9px;
		height: 9px;
		transform: rotate(45deg);
		background: inherit;
		border-left: 1px solid var(--line);
		border-top: 1px solid var(--line);
	}
	@keyframes callout-in {
		from {
			opacity: 0;
			transform: translateY(-4px);
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.advanced-switch.hint::before,
		.advanced-switch.hint::after,
		.callout {
			animation: none;
		}
	}
	.page-link {
		display: flex;
		align-items: center;
		gap: 8px;
		height: 30px;
		border-radius: 4px;
		padding: 0 8px;
		text-align: left;
		font-size: 13px;
		color: var(--text-muted);
		transition:
			background-color var(--dur-fast) var(--ease-out),
			color var(--dur-fast) var(--ease-out);
	}
	.page-link:hover {
		background: var(--surface-raised);
		color: var(--text);
	}
	.page-link.active {
		background: var(--surface-selected);
		color: var(--text);
	}
	.page-link.empty {
		opacity: 0.45;
	}
	.page-link:focus-visible {
		outline: 2px solid var(--focus);
		outline-offset: -2px;
	}
</style>
