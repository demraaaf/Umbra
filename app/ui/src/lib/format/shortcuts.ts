// Every mouse and keyboard control, in one place: the Help page lists them all and the
// editor's shortcut popover shows its own group. Keep this in step with the handlers.
// Within one key, two spaces separate a set of keys ("J  K  L"); one space is part of a name.

export interface Shortcut {
	/** Alternative combinations; each is drawn as keycaps joined with "+". */
	combos: string[][];
	what: string;
}

export interface ShortcutGroup {
	id: 'library' | 'player' | 'editor' | 'window';
	title: string;
	items: Shortcut[];
}

const secs = (n: number) => `${Number.isInteger(n) ? n : n.toFixed(1)} ${n === 1 ? 'second' : 'seconds'}`;

export function libraryShortcuts(): Shortcut[] {
	return [
		{ combos: [['Click']], what: 'Open the clip in the player, or the screenshot in the viewer' },
		{ combos: [['Double-click']], what: 'Open the clip straight in the editor' },
		{ combos: [['Middle click']], what: 'Select or unselect a clip' },
		{
			combos: [['Hold middle button', 'sweep']],
			what: 'Select every clip the pointer passes over. Starting on a selected clip unselects instead.'
		},
		{ combos: [['Ctrl', 'Click']], what: 'Add a clip to the selection, or take it out' },
		{ combos: [['Shift', 'Click']], what: 'Select every clip between the last one picked and this one' },
		{ combos: [['Ctrl', 'A']], what: 'Select every clip shown' },
		{ combos: [['Esc']], what: 'Clear the selection' },
		{ combos: [['←  →  ↑  ↓']], what: 'Move between clips' },
		{ combos: [['Enter']], what: 'Open the focused clip' },
		{ combos: [['/'], ['Ctrl', 'F']], what: 'Search' }
	];
}

export function playerShortcuts(seekSecs: number): Shortcut[] {
	return [
		{ combos: [['Space'], ['K']], what: 'Play or pause' },
		{ combos: [['←  →']], what: `Back or forward ${secs(seekSecs)}` },
		{ combos: [['Shift', '←  →']], what: 'Previous or next frame' },
		{ combos: [['Ctrl', '←  →']], what: 'Previous or next clip in the list' },
		{ combos: [['Home'], ['End']], what: 'Go to the start or the end' },
		{ combos: [['[  ]']], what: 'Previous or next bookmark in a recording' },
		{ combos: [['M']], what: 'Mute or unmute' },
		{ combos: [['E'], ['Enter']], what: 'Open in the editor at the current frame' },
		{ combos: [['Delete']], what: 'Delete the clip and go to the next one (Undo for 5 seconds)' },
		{ combos: [['Esc']], what: 'Back to the library' }
	];
}

export function editorShortcuts(seekSecs: number): Shortcut[] {
	return [
		{ combos: [['Space']], what: 'Play or pause' },
		{ combos: [['J  K  L']], what: 'Shuttle back, pause, forward (press again to go faster)' },
		{ combos: [['←  →']], what: `Back or forward ${secs(seekSecs)}` },
		{ combos: [[',  .']], what: 'Previous or next frame' },
		{ combos: [['Shift', '←  →']], what: `Select from the playhead, ${secs(seekSecs)} at a time` },
		{ combos: [['Shift', ',  .']], what: 'Grow or shrink the selection by one frame' },
		{ combos: [['Shift', 'Drag']], what: 'Select a range on the timeline' },
		{ combos: [['Delete']], what: 'Cut the selection out, or the part between cut points under the playhead' },
		{ combos: [['C']], what: 'Set a cut point at the playhead, or take the one there away' },
		{ combos: [['Q  W']], what: 'Remove everything before or after the playhead (or its part between cut points)' },
		{ combos: [['S  F  N']], what: 'Slow motion, fast or normal speed for the selection or the part under the playhead' },
		{ combos: [['Right-click']], what: 'Speeds and cuts for the part of the timeline clicked' },
		{ combos: [['Ctrl', 'Z'], ['Ctrl', 'Y']], what: 'Undo or redo a trim, cut or speed change' },
		{ combos: [['Alt', 'Drag']], what: 'Drag on the timeline without snapping' },
		{ combos: [['I  O']], what: 'Set the start or end at the playhead' },
		{ combos: [['Home  End']], what: 'Jump to the start or end of the trim' },
		{ combos: [['[  ]']], what: 'Previous or next bookmark in a recording' },
		{ combos: [['Ctrl', 'Wheel']], what: 'Zoom the timeline' },
		{ combos: [['Esc']], what: 'Clear the selection, or go back to the library' }
	];
}

export function windowShortcuts(): Shortcut[] {
	return [
		{ combos: [['Ctrl', 'B']], what: 'Show or hide the sidebar' },
		{ combos: [['Ctrl', 'Alt', 'B']], what: 'Show or hide the recording panel' },
		{ combos: [['Double-click title bar']], what: 'Maximize or restore the window' }
	];
}

export function shortcutGroups(seekSecs: number): ShortcutGroup[] {
	return [
		{ id: 'library', title: 'Clips and albums', items: libraryShortcuts() },
		{ id: 'player', title: 'Player', items: playerShortcuts(seekSecs) },
		{ id: 'editor', title: 'Editor', items: editorShortcuts(seekSecs) },
		{ id: 'window', title: 'Window', items: windowShortcuts() }
	];
}
