<script lang="ts">
	import { onMount } from 'svelte';
	import { api } from '$lib/api';
	import type { CaptureTarget } from '$lib/api/types';
	import { cn } from '$lib/utils';

	/**
	 * A live still of a monitor or window. It loads only once it scrolls into view and refreshes
	 * every 5 seconds while it stays visible and the app is showing.
	 */
	let {
		target,
		maxWidth = 240,
		fit = 'cover',
		empty = 'No preview',
		class: className
	}: {
		target: CaptureTarget;
		maxWidth?: number;
		fit?: 'cover' | 'contain';
		/** Shown when the source has nothing to show, such as a minimised window. */
		empty?: string;
		class?: string;
	} = $props();

	const REFRESH_MS = 5000;

	let box = $state<HTMLSpanElement | null>(null);
	let preview = $state<string | null>(null);
	let unavailable = $state(false);

	onMount(() => {
		let visible = false;
		let timer: ReturnType<typeof setInterval> | null = null;
		let alive = true;

		async function load() {
			if (!visible || document.hidden) return;
			try {
				const url = await api.getPreview(target, maxWidth);
				if (!alive) return;
				preview = url;
				unavailable = url === null;
			} catch {
				if (alive) unavailable = true;
			}
		}

		function start() {
			if (timer) return;
			void load();
			timer = setInterval(load, REFRESH_MS);
		}
		function stop() {
			if (timer) clearInterval(timer);
			timer = null;
		}

		const observer = new IntersectionObserver((entries) => {
			visible = entries.some((e) => e.isIntersecting);
			if (visible) start();
			else stop();
		});
		if (box) observer.observe(box);
		const onVisibility = () => (document.hidden ? stop() : visible && start());
		document.addEventListener('visibilitychange', onVisibility);

		return () => {
			alive = false;
			stop();
			observer.disconnect();
			document.removeEventListener('visibilitychange', onVisibility);
		};
	});
</script>

<span bind:this={box} class={cn('relative block overflow-hidden bg-raised', className)}>
	{#if preview}
		<img src={preview} alt="" class="still size-full {fit === 'contain' ? 'object-contain' : 'object-cover'}" draggable="false" />
	{:else if unavailable}
		<span class="flex size-full items-center justify-center text-xs text-subtle">{empty}</span>
	{/if}
</span>

<style>
	.still {
		animation: appear 160ms var(--ease-out);
	}
	@keyframes appear {
		from {
			opacity: 0;
		}
	}
</style>
