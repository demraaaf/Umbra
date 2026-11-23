<script lang="ts">
	import { stepBookmark } from '$lib/format/bookmarks';
	import { Button } from '$lib/components/ui/button';
	import * as Empty from '$lib/components/ui/empty';
	import ClipHeader from '$lib/components/app/ClipHeader.svelte';
	import Inspector from '$lib/components/app/editor/Inspector.svelte';
	import PlayerStage from '$lib/components/app/editor/PlayerStage.svelte';
	import Timeline from '$lib/components/app/editor/Timeline.svelte';
	import Transport from '$lib/components/app/editor/Transport.svelte';
	import SequenceStrip from '$lib/components/app/editor/SequenceStrip.svelte';
	import { getSession } from '$lib/editor/session.svelte';
	import { fadeAt, getSequence } from '$lib/editor/sequence.svelte';
	import { app, closeEditor, currentClip } from '$lib/stores/app.svelte';

	const clip = $derived(currentClip());
	/** The clips in this edit; the editor works on one of them at a time. */
	const sequence = $derived(clip ? getSequence(clip.path) : null);
	const main = $derived(clip ? getSession(clip) : null);
	const session = $derived(sequence?.activeEntry?.session ?? main);

	// Keep the session pointed at the latest clip record (thumbnails arrive after the fact).
	$effect(() => {
		if (main && clip && main.clip !== clip) main.rebind(clip);
	});

	// Play all: at the end of one clip, go on with the next.
	$effect(() => {
		if (!sequence) return;
		const entries = sequence.entries;
		entries.forEach((entry, i) => {
			entry.session.playback.onReachedEnd = () => {
				const next = entries[i + 1];
				if (!sequence.playAll || !next) {
					sequence.playAll = false;
					return;
				}
				sequence.select(i + 1);
				next.session.playback.seek(next.session.inSecs);
				next.session.playback.playWhenReady();
			};
		});
	});

	/** The fade at the playhead, as the export will have it. */
	const fade = $derived.by(() => {
		if (!sequence || !main || !session) return 1;
		const index = sequence.entries.findIndex((e) => e.session === session);
		if (index < 0) return 1;
		const offsets = sequence.offsets;
		const many = offsets.length > 1;
		const t = offsets[index] + session.outputTimeOf(session.playback.time);
		return fadeAt({ ...main.fades, between: many ? main.fades.between : 0 }, t, sequence.totalSecs, offsets.slice(1));
	});

	// Opened from the player: start where the player was.
	$effect(() => {
		if (!session || app.editorSeek === null) return;
		const at = app.editorSeek;
		app.editorSeek = null;
		session.playback.seek(at);
	});

	// Waveforms under the timeline only when the editor is big enough to spare the room.
	const WAVE_ROW = 28;
	let height = $state(0);
	let width = $state(0);
	const showWaves = $derived.by(() => {
		const rows = clip?.audioTracks.length ?? 0;
		if (rows === 0 || rows > 10 || width < 1100) return false;
		return height - rows * WAVE_ROW >= 720;
	});

	// Leaving the editor stops playback; the session itself is kept for when you come back.
	$effect(() => {
		const current = session;
		return () => current?.playback.pause();
	});

	/** Keys that belong to the editor, unless the user is typing or operating another control. */
	function onKeydown(e: KeyboardEvent) {
		if (!session || e.defaultPrevented || e.metaKey) return;
		const target = e.target as HTMLElement | null;
		if (
			target?.closest(
				'input, textarea, select, [contenteditable="true"], [role="spinbutton"], [role="combobox"], [role="listbox"], [role="menu"], [role="dialog"], [role="alertdialog"]'
			)
		)
			return;
		// Undo and redo of trims and cuts.
		if (e.ctrlKey && !e.altKey) {
			const redo = e.code === 'KeyY' || (e.code === 'KeyZ' && e.shiftKey);
			if (e.code === 'KeyZ' && !e.shiftKey) {
				e.preventDefault();
				session.undo();
			} else if (redo) {
				e.preventDefault();
				session.redo();
			}
			return;
		}
		if (e.ctrlKey || e.altKey) return;
		const onSlider = !!target?.closest('[role="slider"]');
		const playback = session.playback;

		const seekSecs = session.seekSecs;
		switch (e.code) {
			case 'Space':
				if (target?.closest('button, a, summary, [role="button"], [role="switch"], [role="checkbox"], [role="radio"]')) return;
				e.preventDefault();
				playback.toggle();
				break;
			case 'KeyK':
				e.preventDefault();
				playback.pause();
				break;
			case 'KeyL':
				e.preventDefault();
				playback.shuttleForward();
				break;
			case 'KeyJ':
				e.preventDefault();
				playback.shuttleReverse();
				break;
			case 'KeyI':
				if (session.busy) return;
				e.preventDefault();
				session.setInAtPlayhead();
				break;
			case 'KeyO':
				if (session.busy) return;
				e.preventDefault();
				session.setOutAtPlayhead();
				break;
			case 'ArrowLeft':
			case 'ArrowRight': {
				if (onSlider) return;
				e.preventDefault();
				const dir = e.code === 'ArrowLeft' ? -1 : 1;
				if (e.shiftKey) session.selectBy(dir * seekSecs);
				else playback.skip(dir * seekSecs);
				break;
			}
			case 'Comma':
			case 'Period': {
				if (onSlider) return;
				e.preventDefault();
				const dir = e.code === 'Comma' ? -1 : 1;
				if (e.shiftKey) session.selectBy(dir * session.frameSecs);
				else playback.step(dir);
				break;
			}
			case 'Delete':
			case 'Backspace':
				if (session.selection) {
					e.preventDefault();
					session.cutSelection();
				} else if (session.deleteSegment()) {
					e.preventDefault();
				}
				break;
			case 'KeyC':
				if (session.busy) return;
				e.preventDefault();
				session.toggleCutAtPlayhead();
				break;
			case 'KeyQ':
				if (session.busy) return;
				e.preventDefault();
				session.deleteBefore();
				break;
			case 'KeyS':
			case 'KeyF':
			case 'KeyN':
				if (session.busy) return;
				e.preventDefault();
				session.speedHere(e.code === 'KeyS' ? 0.5 : e.code === 'KeyF' ? 2 : 1);
				break;
			case 'KeyW':
				if (session.busy) return;
				e.preventDefault();
				session.deleteAfter();
				break;
			case 'Home':
				if (onSlider) return;
				e.preventDefault();
				playback.seek(session.inSecs);
				break;
			case 'BracketLeft':
			case 'BracketRight': {
				const to = stepBookmark(session.clip.bookmarks, playback.time, e.code === 'BracketLeft' ? -1 : 1);
				if (to === null) return;
				e.preventDefault();
				playback.seek(to);
				break;
			}
			case 'End':
				if (onSlider) return;
				e.preventDefault();
				playback.seek(session.outSecs);
				break;
			case 'Escape':
				e.preventDefault();
				if (session.selection) session.clearSelection();
				else closeEditor();
				break;
		}
	}
</script>

<svelte:window onkeydown={onKeydown} />

{#if session && clip && main && sequence}
	<section class="flex min-h-0 flex-1 flex-col" aria-label="Clip editor" bind:clientHeight={height} bind:clientWidth={width}>
		<ClipHeader clip={session.clip} busy={session.busy || main.busy} />
		<div class="flex min-h-0 flex-1">
			<PlayerStage {session} {fade} />
			<Inspector {session} {main} {sequence} />
		</div>
		<div class="@container shrink-0 border-t border-line bg-panel pb-2">
			<Transport {session} />
			<Timeline {session} {showWaves} />
			{#if sequence.items.length > 1}
				<SequenceStrip {sequence} busy={main.busy} />
			{/if}
		</div>
	</section>
{:else}
	<Empty.Root class="flex-1 border-0">
		<Empty.Header>
			<Empty.Title>No clip is open</Empty.Title>
			<Empty.Description>Pick a clip in the library to trim, crop and export it.</Empty.Description>
		</Empty.Header>
		<Empty.Content>
			<Button variant="outline" onclick={closeEditor}>Go to clips</Button>
		</Empty.Content>
	</Empty.Root>
{/if}
