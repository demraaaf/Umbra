// Sidebar visibility, remembered between sessions. Ctrl+B toggles the left sidebar and
// Ctrl+Alt+B the recording panel on the right.

const KEY = 'umbra.layout.v1';

function restore(): { leftCollapsed: boolean; rightCollapsed: boolean } {
	try {
		const raw = JSON.parse(localStorage.getItem(KEY) ?? '{}') as Partial<{ leftCollapsed: boolean; rightCollapsed: boolean }>;
		return { leftCollapsed: raw.leftCollapsed === true, rightCollapsed: raw.rightCollapsed === true };
	} catch {
		return { leftCollapsed: false, rightCollapsed: false };
	}
}

export const layout = $state({
	...restore(),
	/** The recording panel was folded away for the editor, not by the user. */
	rightAuto: false
});

function persist() {
	try {
		// A panel folded away only for the editor is remembered as open.
		const right = layout.rightCollapsed && !layout.rightAuto;
		localStorage.setItem(KEY, JSON.stringify({ leftCollapsed: layout.leftCollapsed, rightCollapsed: right }));
	} catch {
		/* storage may be unavailable */
	}
}

/**
 * Folds the recording panel away while the editor is shown and brings it back after, unless
 * the user had folded it away already (then it stays folded) or changed it in the editor.
 */
export function followScreen(screen: string) {
	if (screen === 'editor') {
		if (!layout.rightCollapsed) {
			layout.rightCollapsed = true;
			layout.rightAuto = true;
		}
	} else if (layout.rightAuto) {
		layout.rightCollapsed = false;
		layout.rightAuto = false;
	}
}

export function toggleLeft() {
	layout.leftCollapsed = !layout.leftCollapsed;
	persist();
}

export function toggleRight() {
	layout.rightCollapsed = !layout.rightCollapsed;
	// The user decided; leaving the editor no longer changes it back.
	layout.rightAuto = false;
	persist();
}

export const SHORTCUT_LEFT = 'Ctrl+B';
export const SHORTCUT_RIGHT = 'Ctrl+Alt+B';
