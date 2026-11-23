<script lang="ts">
	import type { Component } from 'svelte';
	import VideoIcon from '@lucide/svelte/icons/video';
	import KeyboardIcon from '@lucide/svelte/icons/keyboard';
	import StampIcon from '@lucide/svelte/icons/stamp';
	import type { Clip } from '$lib/api/types';

	/** Icons for the layers Umbra draws over a clip (camera, keyboard and mouse, logo); nothing without any. */
	let { clip, class: className = '' }: { clip: Clip; class?: string } = $props();

	const layers = $derived.by(() => {
		const list: { icon: Component; label: string }[] = [];
		if (clip.camera) list.push({ icon: VideoIcon, label: 'Camera' });
		if (clip.input) list.push({ icon: KeyboardIcon, label: 'Keyboard and mouse' });
		if (clip.logo) list.push({ icon: StampIcon, label: 'Logo' });
		return list;
	});
	const text = $derived(`Layers: ${layers.map((l) => l.label.toLowerCase()).join(', ')}`);
</script>

{#if layers.length > 0}
	<span class="flex items-center gap-1 {className}" role="img" aria-label={text} title={text}>
		{#each layers as layer (layer.label)}
			<layer.icon class="size-3" aria-hidden="true" />
		{/each}
	</span>
{/if}
