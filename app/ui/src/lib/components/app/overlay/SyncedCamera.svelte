<script lang="ts">
	import type { Playback } from '$lib/editor/playback.svelte';

	/**
	 * A clip's camera file, kept in step with the clip's playback. The file starts at `offset`
	 * seconds of the clip. It is silent and follows the clip's time: it plays along while the
	 * clip plays forward, and is moved to the clip's frame otherwise (paused, scrubbing,
	 * playing backwards).
	 */
	let { url, offset, playback }: { url: string; offset: number; playback: Playback } = $props();

	let video = $state<HTMLVideoElement | null>(null);
	/** Set once the file can be moved around in. */
	let ready = $state(false);

	/** Further apart than this while playing, the camera is moved to the clip's time. */
	const DRIFT = 0.12;

	$effect(() => {
		const v = video;
		if (!v || !ready) return;
		const target = Math.max(0, playback.time - offset);
		const rate = playback.speed * playback.speedAt(playback.time);
		if (playback.playing && rate > 0 && !playback.buffering) {
			if (Math.abs(v.playbackRate - rate) > 1e-3) v.playbackRate = rate;
			if (Math.abs(v.currentTime - target) > DRIFT) v.currentTime = target;
			if (v.paused) void v.play().catch(() => {});
		} else {
			if (!v.paused) v.pause();
			if (Math.abs(v.currentTime - target) > 0.5 / playback.fps) v.currentTime = target;
		}
	});
</script>

<!-- svelte-ignore a11y_media_has_caption -->
<video
	bind:this={video}
	src={url}
	class="block size-full bg-black object-fill"
	preload="auto"
	muted
	playsinline
	disablepictureinpicture
	disableremoteplayback
	aria-hidden="true"
	onloadedmetadata={() => (ready = true)}
></video>
