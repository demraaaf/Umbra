<script lang="ts">
	import { Button } from '$lib/components/ui/button';
	import * as Empty from '$lib/components/ui/empty';
	import * as ToggleGroup from '$lib/components/ui/toggle-group';
	import ClipHeader from '$lib/components/app/ClipHeader.svelte';
	import { backToLibrary, browseStep, deleteWithUndo, viewerClip } from '$lib/stores/app.svelte';

	const clip = $derived(viewerClip());

	/** `fit` shows the whole image; `actual` shows it pixel for pixel, scrollable. */
	let zoom = $state<'fit' | 'actual'>('fit');
	let failed = $state(false);

	$effect(() => {
		void clip?.videoUrl;
		failed = false;
	});

	function onKeydown(e: KeyboardEvent) {
		if (e.defaultPrevented || e.metaKey || e.altKey) return;
		const target = e.target as HTMLElement | null;
		if (target?.closest('input, textarea, [role="dialog"], [role="alertdialog"], [role="menu"], [role="radiogroup"]')) return;
		// The arrow keys (with or without Ctrl, as in the player) step through the list.
		if (e.code === 'ArrowLeft' || e.code === 'ArrowRight') {
			e.preventDefault();
			browseStep(e.code === 'ArrowLeft' ? -1 : 1);
			return;
		}
		if (e.ctrlKey) return;
		if (e.key === 'Escape') {
			e.preventDefault();
			backToLibrary();
		} else if (e.code === 'KeyF') {
			e.preventDefault();
			zoom = zoom === 'fit' ? 'actual' : 'fit';
		} else if (e.code === 'Delete' && clip) {
			e.preventDefault();
			deleteWithUndo(clip.path);
		}
	}
</script>

<svelte:window onkeydown={onKeydown} />

{#if clip}
	<section class="flex min-h-0 flex-1 flex-col" aria-label="Screenshot viewer">
		<ClipHeader {clip}>
			{#snippet tools()}
				<ToggleGroup.Root
					type="single"
					variant="outline"
					size="sm"
					value={zoom}
					onValueChange={(v) => v && (zoom = v as 'fit' | 'actual')}
					aria-label="Zoom (F)"
				>
					<ToggleGroup.Item value="fit" class="px-2" title="Fit to the window (F)">Fit</ToggleGroup.Item>
					<ToggleGroup.Item value="actual" class="tnum px-2" title="Actual pixels (F)">100%</ToggleGroup.Item>
				</ToggleGroup.Root>
			{/snippet}
		</ClipHeader>

		<div class="stage min-h-0 flex-1 bg-black" class:actual={zoom === 'actual'}>
			{#if failed}
				<p class="m-auto max-w-xs p-6 text-center text-sm text-muted-foreground" role="alert">
					The image could not be shown. The file may have been moved or deleted.
				</p>
			{:else}
				<img
					src={clip.videoUrl}
					alt={clip.fileName}
					style:width={zoom === 'actual' ? `${clip.width}px` : null}
					style:height={zoom === 'actual' ? `${clip.height}px` : null}
					draggable="false"
					onerror={() => (failed = true)}
				/>
			{/if}
		</div>
	</section>
{:else}
	<Empty.Root class="flex-1 border-0">
		<Empty.Header>
			<Empty.Title>No screenshot is open</Empty.Title>
			<Empty.Description>Pick a screenshot in the library to view it.</Empty.Description>
		</Empty.Header>
		<Empty.Content>
			<Button variant="outline" onclick={backToLibrary}>Go to clips</Button>
		</Empty.Content>
	</Empty.Root>
{/if}

<style>
	.stage {
		display: flex;
		overflow: hidden;
		padding: 16px;
	}
	.stage img {
		width: 100%;
		height: 100%;
		object-fit: contain;
	}
	.stage.actual {
		overflow: auto;
		padding: 0;
	}
	.stage.actual img {
		max-width: none;
		margin: auto;
		flex-shrink: 0;
		object-fit: fill;
	}
</style>
