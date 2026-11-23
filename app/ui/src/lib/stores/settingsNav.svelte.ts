// Which Settings page is open, and the settings search. Shared by the Settings screen and the
// sidebar, which lists the pages under Settings while it is open.

export interface SettingsPage {
	id: string;
	label: string;
}

export const SETTINGS_GROUPS: { label: string; pages: SettingsPage[] }[] = [
	{
		label: 'Recording',
		pages: [
			{ id: 'capture', label: 'Capture' },
			{ id: 'video', label: 'Video' },
			{ id: 'audio', label: 'Audio' },
			{ id: 'overlay', label: 'Overlays' },
			{ id: 'clip', label: 'Clip' },
			{ id: 'autoclip', label: 'Auto-clip' },
			{ id: 'screenshots', label: 'Screenshots' },
			{ id: 'recording', label: 'Recording' }
		]
	},
	{
		label: 'Library',
		pages: [
			{ id: 'player', label: 'Player and editor' },
			{ id: 'storage', label: 'Storage' }
		]
	},
	{
		label: 'Umbra',
		pages: [
			{ id: 'help', label: 'Help' },
			{ id: 'about', label: 'About' }
		]
	}
];

export const SETTINGS_PAGES = SETTINGS_GROUPS.flatMap((g) => g.pages);

const PAGE_KEY = 'umbra.settings.page';
const ADVANCED_KEY = 'umbra.settings.advanced';
const HINT_KEY = 'umbra.settings.advancedTried';

function savedPage(): string {
	try {
		const id = localStorage.getItem(PAGE_KEY);
		return id && SETTINGS_PAGES.some((p) => p.id === id) ? id : 'capture';
	} catch {
		return 'capture';
	}
}

function saved(key: string): boolean {
	try {
		return localStorage.getItem(key) === '1';
	} catch {
		return false;
	}
}

export const settingsNav = $state({
	page: savedPage(),
	/** Shows the settings most people never need. Off by default. */
	advanced: saved(ADVANCED_KEY),
	/** The switch draws attention until it has been turned on and off once. */
	advancedHint: !saved(HINT_KEY),
	query: '',
	/** Matching settings per page while searching. */
	counts: {} as Record<string, number>
});

/** Opens a Settings page (the screen itself is opened by the caller) and ends a search. */
export function openSettingsPage(id: string) {
	settingsNav.query = '';
	settingsNav.page = id;
	try {
		localStorage.setItem(PAGE_KEY, id);
	} catch {
		// The page just opens at Capture next time.
	}
}

export function setAdvanced(on: boolean) {
	settingsNav.advanced = on;
	// It starts off, so turning it off means it has been on: the hint has done its job.
	if (!on) settingsNav.advancedHint = false;
	try {
		localStorage.setItem(ADVANCED_KEY, on ? '1' : '0');
		if (!on) localStorage.setItem(HINT_KEY, '1');
	} catch {
		// It just starts off next time.
	}
}

/** A first setup starts simple, with the hint on the switch, whatever this webview remembers. */
export function resetAdvanced() {
	setAdvanced(false);
	settingsNav.advancedHint = true;
	try {
		localStorage.removeItem(HINT_KEY);
	} catch {
		// The hint just stays off.
	}
}

/** Advanced settings show when switched on, and while searching so a search finds everything. */
export function showAdvanced(): boolean {
	return settingsNav.advanced || settingsNav.query.trim() !== '';
}
