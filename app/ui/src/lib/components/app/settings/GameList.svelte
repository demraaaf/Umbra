<script lang="ts">
	import { onMount } from 'svelte';
	import { api } from '$lib/api';
	import type { RunningGame } from '$lib/api/types';
	import { app } from '$lib/stores/app.svelte';
	import ExeList from './ExeList.svelte';

	/**
	 * A list of games: an ExeList that offers, with one click, the games running now and the
	 * games your clips were recorded in.
	 */
	let {
		items = $bindable(),
		inputId,
		emptyText,
		paths = false,
		folders = false
	}: {
		items: string[];
		inputId: string;
		emptyText: string;
		paths?: boolean;
		folders?: boolean;
	} = $props();

	/** At most this many games from the clips, the most recently played first. */
	const FROM_CLIPS = 10;

	let running = $state<RunningGame[]>([]);

	async function look() {
		try {
			running = await api.listRunningGames();
		} catch {
			running = [];
		}
	}

	onMount(() => {
		void look();
		// A game started while the settings are open shows up too.
		const timer = setInterval(look, 5000);
		return () => clearInterval(timer);
	});

	const fromClips = $derived.by(() => {
		const seen = new Map<string, { exe: string; name: string; at: string }>();
		for (const clip of app.clips) {
			const exe = clip.details?.game_exe;
			if (!exe) continue;
			const key = exe.toLowerCase();
			const known = seen.get(key);
			if (!known || clip.createdAt > known.at) {
				seen.set(key, { exe, name: clip.details.game || clip.game || exe.replace(/\.exe$/i, ''), at: clip.createdAt });
			}
		}
		return [...seen.values()].sort((a, b) => b.at.localeCompare(a.at)).slice(0, FROM_CLIPS);
	});

	const suggestions = $derived.by(() => {
		const out: { exe: string; name: string; group: string }[] = running.map((g) => ({ ...g, group: 'Running now' }));
		for (const g of fromClips) {
			if (!out.some((o) => o.exe.toLowerCase() === g.exe.toLowerCase())) out.push({ exe: g.exe, name: g.name, group: 'From your clips' });
		}
		return out;
	});
	const names = $derived(Object.fromEntries(suggestions.map((s) => [s.exe.toLowerCase(), s.name])));
</script>

<ExeList bind:items {inputId} {suggestions} {names} {emptyText} {paths} {folders} />
