<script lang="ts">
	import Volume2Icon from '@lucide/svelte/icons/volume-2';
	import VolumeXIcon from '@lucide/svelte/icons/volume-x';
	import type { ClipAudioTrack } from '$lib/api/types';
	import { Slider } from '$lib/components/ui/slider';
	import IconButton from '$lib/components/app/IconButton.svelte';
	import NumberField from '$lib/components/app/fields/NumberField.svelte';
	import type { EditSession } from '$lib/editor/session.svelte';

	/** One audio track: mute, a 0 to 200 % slider and a typed percentage. 0 % leaves it out. */
	let { session, track }: { session: EditSession; track: ClipAudioTrack } = $props();

	const muted = $derived(session.isMuted(track.index));
	const percent = $derived(Math.round(session.volumeOf(track.index) * 100));
	const out = $derived(session.effectiveVolume(track.index) === 0);
	const hint = $derived.by(() => {
		if (track.role === 'mix') return 'What most players play. Rebuilt on export from the tracks in it, at their volumes.';
		const hasMix = session.clip.audioTracks.some((t) => t.role === 'mix');
		if (track.role === 'separate' && hasMix) return 'Only on its own track, not in Mix.';
		if (track.role === 'inOthers')
			return session.isUnmixed(track.index)
				? 'Also changes this app inside Other apps and Mix.'
				: 'Also inside Other apps, but silent or not found there, so this only changes its own track.';
		return null;
	});
</script>

<li class="flex flex-col gap-1 px-2 py-2">
	<div class="flex items-center gap-2">
		<span class="tnum w-4 text-xs text-subtle" aria-hidden="true">{track.index + 1}</span>
		<span class="min-w-0 flex-1 truncate text-sm {out ? 'text-subtle line-through' : 'text-foreground'}">{track.title}</span>
		<IconButton
			label="Mute {track.title}"
			icon={out ? VolumeXIcon : Volume2Icon}
			size="icon-xs"
			pressed={out}
			disabled={session.busy}
			onclick={() => session.toggleTrack(track.index)}
		/>
	</div>
	{#if hint}
		<p class="pl-6 text-xs text-subtle">{hint}</p>
	{/if}
	<div class="flex items-center gap-2 pl-6">
		<Slider
			type="single"
			min={0}
			max={200}
			step={5}
			value={muted ? 0 : percent}
			onValueChange={(v) => session.setTrackVolume(track.index, v / 100)}
			disabled={session.busy}
			aria-label="{track.title} volume"
		/>
		<NumberField
			label="{track.title} volume in percent"
			unit="%"
			min={0}
			max={200}
			step={5}
			value={muted ? 0 : percent}
			oncommit={(v) => session.setTrackVolume(track.index, v / 100)}
			disabled={session.busy}
			class="w-20 shrink-0"
		/>
	</div>
</li>
