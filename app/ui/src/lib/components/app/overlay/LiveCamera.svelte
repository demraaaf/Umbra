<script lang="ts">
	import CameraStandIn from './CameraStandIn.svelte';

	/**
	 * The camera called `name`, live, through the browser's own camera access (Windows asks
	 * once whether Umbra may use it). Only for previews; recording goes through the engine.
	 * Shows a stand-in while the camera can't be opened.
	 */
	let { name }: { name: string } = $props();

	let video = $state<HTMLVideoElement | null>(null);
	let stream = $state.raw<MediaStream | null>(null);
	let failed = $state(false);

	async function open(wanted: string): Promise<MediaStream> {
		const media = navigator.mediaDevices;
		// Labels are empty until camera access is granted, so ask once to learn them.
		let devices = await media.enumerateDevices();
		if (!devices.some((d) => d.kind === 'videoinput' && d.label)) {
			const probe = await media.getUserMedia({ video: true });
			probe.getTracks().forEach((t) => t.stop());
			devices = await media.enumerateDevices();
		}
		const match = devices.find((d) => d.kind === 'videoinput' && d.label.includes(wanted));
		if (!match) throw new Error('camera not found');
		return media.getUserMedia({ video: { deviceId: { exact: match.deviceId }, width: { ideal: 1280 } } });
	}

	$effect(() => {
		const wanted = name;
		let opened: MediaStream | null = null;
		let alive = true;
		failed = false;
		if (!wanted || !navigator.mediaDevices?.getUserMedia) {
			failed = true;
			return;
		}
		open(wanted)
			.then((s) => {
				opened = s;
				if (alive) stream = s;
				else s.getTracks().forEach((t) => t.stop());
			})
			.catch(() => {
				if (alive) failed = true;
			});
		return () => {
			alive = false;
			opened?.getTracks().forEach((t) => t.stop());
			stream = null;
		};
	});

	$effect(() => {
		if (video) video.srcObject = stream;
	});
</script>

{#if failed}
	<CameraStandIn />
{:else}
	<video bind:this={video} class="block size-full bg-black object-fill" autoplay muted playsinline disablepictureinpicture aria-hidden="true"></video>
{/if}
