// Dragging clips out of Umbra into another program (Discord, Explorer, a browser upload).
//
// Pressing on a clip and moving a few pixels hands the file to Windows' own drag and drop;
// when the clip is selected, every selected clip goes along. A plain click still opens it.
// While Umbra drags its own clips, its "drop to import" overlay stays away.

import { toast } from 'svelte-sonner';
import { api } from '$lib/api';
import { app } from '$lib/stores/app.svelte';
import { importer } from '$lib/stores/importer.svelte';
import { selection } from '$lib/stores/selection.svelte';

const THRESHOLD = 6;

export function dragOut(node: HTMLElement, path: string) {
	let current = path;
	let start: { x: number; y: number } | null = null;
	/** The click that ends a drag must not open the clip. */
	let swallowClick = false;

	const down = (e: PointerEvent) => {
		if (e.button !== 0 || e.ctrlKey || e.shiftKey || e.metaKey || e.altKey) return;
		start = { x: e.clientX, y: e.clientY };
	};
	const move = async (e: PointerEvent) => {
		if (!start) return;
		if (!(e.buttons & 1)) {
			start = null;
			return;
		}
		if (Math.hypot(e.clientX - start.x, e.clientY - start.y) < THRESHOLD) return;
		start = null;
		swallowClick = true;
		const paths = selection.paths.includes(current) ? [...selection.paths] : [current];
		importer.ownDrag = true;
		try {
			const dropped = await api.dragClipsOut(paths);
			// The raw file is dragged: its camera and logo are drawn in only by an export or a copy.
			const overlaid = paths.some((p) => {
				const clip = app.clips.find((c) => c.path === p);
				return clip && (clip.camera || clip.details.logo);
			});
			if (dropped && overlaid) {
				toast.info('The camera and logo are not in the dragged file', {
					description: 'They have to be drawn in first: export the clip, or right-click it, choose Copy file and paste it instead.'
				});
			}
		} catch {
			// Nothing to tell: the drag simply did not happen.
		} finally {
			importer.ownDrag = false;
			importer.dragging = null;
			setTimeout(() => (swallowClick = false), 250);
		}
	};
	const up = () => {
		start = null;
	};
	const click = (e: MouseEvent) => {
		if (!swallowClick) return;
		swallowClick = false;
		e.preventDefault();
		e.stopPropagation();
	};
	// The browser's own drag of the thumbnail picture would get in the way.
	const nativeDrag = (e: DragEvent) => e.preventDefault();

	node.addEventListener('pointerdown', down);
	node.addEventListener('pointermove', move);
	node.addEventListener('pointerup', up);
	node.addEventListener('click', click, true);
	node.addEventListener('dragstart', nativeDrag);
	return {
		update(next: string) {
			current = next;
		},
		destroy() {
			node.removeEventListener('pointerdown', down);
			node.removeEventListener('pointermove', move);
			node.removeEventListener('pointerup', up);
			node.removeEventListener('click', click, true);
			node.removeEventListener('dragstart', nativeDrag);
		}
	};
}
