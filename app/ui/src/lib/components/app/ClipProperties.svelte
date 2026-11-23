<script lang="ts">
	import FolderOpenIcon from '@lucide/svelte/icons/folder-open';
	import * as Dialog from '$lib/components/ui/dialog';
	import { Button } from '$lib/components/ui/button';
	import { app, closeProperties, revealClip } from '$lib/stores/app.svelte';
	import {
		CODEC_LABELS,
		clipTitle,
		formatBytes,
		formatFullDate,
		formatResolution,
		formatTimecode,
		gameLabel
	} from '$lib/format/format';

	/** Everything known about one clip: the file, the video, how it was recorded, its audio. */

	const clip = $derived(app.clips.find((c) => c.path === app.propertiesPath) ?? null);
	const details = $derived(clip?.details ?? {});
	const folder = $derived(clip ? clip.path.slice(0, Math.max(0, clip.path.lastIndexOf('\\'))) : '');

	const DETECTED: Record<string, string> = {
		'captured window': 'A window was captured, so its program is the game',
		'listed game': 'Found running from the list of games in Settings',
		'fullscreen program': 'Detected as the program filling the screen',
		'no game': 'No game was running; recorded as Desktop'
	};
	const hasOthers = $derived(clip?.audioTracks.some((t) => t.title.toLowerCase() === 'other apps') ?? false);
	function roleLabel(role: string): string {
		if (role === 'mix') return 'Combined track';
		if (role === 'inMix') return 'In the combined track';
		if (role === 'inOthers') return 'Own track, also inside Other apps';
		return hasOthers || clip?.audioTracks.some((t) => t.role === 'mix') ? 'Own track only' : 'Own track';
	}

	function rate(bps: number): string {
		return bps >= 1_000_000 ? `${(bps / 1_000_000).toFixed(1)} Mbit/s` : `${Math.round(bps / 1000)} kbit/s`;
	}

	type Row = [label: string, value: string | null | undefined];
	const sections = $derived.by((): { title: string; rows: Row[] }[] => {
		if (!clip) return [];
		const image = clip.kind === 'image';
		const fps = clip.fps > 0 ? `${Number(clip.fps.toFixed(2))} fps` : null;
		const set = details.recorded_fps && Math.abs(Number(details.recorded_fps) - clip.fps) >= 1 ? ` (set to ${details.recorded_fps})` : '';
		const list: { title: string; rows: Row[] }[] = [
			{
				title: 'File',
				rows: [
					['Name', clip.fileName],
					['Folder', folder],
					['Size', formatBytes(clip.sizeBytes)],
					['Saved', formatFullDate(clip.createdAt)]
				]
			},
			{
				title: image ? 'Image' : 'Video',
				rows: image
					? [['Resolution', formatResolution(clip.width, clip.height)]]
					: [
							['Length', formatTimecode(clip.durationSecs)],
							['Resolution', formatResolution(clip.width, clip.height)],
							['Frame rate', fps ? fps + set : null],
							['Codec', CODEC_LABELS[clip.codec] ?? clip.codec],
							['Average bitrate', clip.bitRate > 0 ? rate(clip.bitRate) : null],
							['Rate control', details.rate],
							['Encoder', details.encoder ? `${details.encoder}${details.preset ? `, preset ${details.preset}` : ''}` : null],
							['Graphics card', details.gpu]
						]
			},
			{
				title: 'Game',
				rows: [
					['Game', details.game ?? gameLabel(clip.game)],
					['Program', details.game_exe],
					['Window', details.window],
					['How it was found', details.detected ? (DETECTED[details.detected] ?? details.detected) : null],
					['Captured', details.source],
					['Recorded with', details.app]
				]
			}
		];
		return list.map((s) => ({ ...s, rows: s.rows.filter(([, v]) => v !== null && v !== undefined && v !== '') }));
	});

	const albums = $derived(clip ? app.albums.filter((a) => clip.albums.includes(a.id)).map((a) => a.name) : []);
	const hasDetails = $derived(Object.keys(details).some((k) => k !== 'game'));
</script>

<Dialog.Root open={clip !== null} onOpenChange={(open) => !open && closeProperties()}>
	<Dialog.Content class="flex max-h-[85vh] flex-col sm:max-w-xl">
		{#if clip}
			<Dialog.Header>
				<Dialog.Title class="truncate pr-6">{clipTitle(clip)}</Dialog.Title>
				<Dialog.Description>Properties</Dialog.Description>
			</Dialog.Header>

			<!-- Only the details scroll; the title and the buttons stay in view. -->
			<div class="-mx-4 flex min-h-0 flex-col gap-5 overflow-y-auto px-4">
				{#each sections as section (section.title)}
					{#if section.rows.length > 0}
						<section aria-label={section.title}>
							<h3 class="mb-2 text-xs font-medium text-muted-foreground">{section.title}</h3>
							<dl class="grid grid-cols-[9rem_minmax(0,1fr)] gap-x-4 gap-y-1.5 text-sm">
								{#each section.rows as [label, value] (label)}
									<dt class="text-muted-foreground">{label}</dt>
									<dd class="selectable tnum min-w-0 break-words text-foreground">{value}</dd>
								{/each}
							</dl>
						</section>
					{/if}
				{/each}

				{#if !hasDetails && clip.kind === 'video'}
					<p class="text-xs text-subtle">
						This clip has no recording details. Umbra stores them in clips saved from now on; older clips and files from
						other programs don't have them.
					</p>
				{/if}

				{#if clip.audioTracks.length > 0}
					<section aria-label="Audio">
						<h3 class="mb-2 text-xs font-medium text-muted-foreground">Audio tracks</h3>
						<ol class="flex flex-col gap-1 text-sm">
							{#each clip.audioTracks as track (track.index)}
								<li class="grid grid-cols-[1.5rem_minmax(0,1fr)_auto] items-baseline gap-2">
									<span class="tnum text-xs text-subtle">{track.index + 1}</span>
									<span class="truncate text-foreground">{track.title}</span>
									<span class="text-xs text-muted-foreground">{roleLabel(track.role)}</span>
								</li>
							{/each}
						</ol>
					</section>
				{/if}

				{#if albums.length > 0}
					<section aria-label="Albums">
						<h3 class="mb-2 text-xs font-medium text-muted-foreground">Albums</h3>
						<p class="text-sm text-foreground">{albums.join(', ')}</p>
					</section>
				{/if}
			</div>

			<Dialog.Footer>
				<Button variant="outline" onclick={() => revealClip(clip.path)}>
					<FolderOpenIcon data-icon="inline-start" />
					Show in folder
				</Button>
				<Button onclick={closeProperties}>Close</Button>
			</Dialog.Footer>
		{/if}
	</Dialog.Content>
</Dialog.Root>
