import { isTauri } from './index';

/**
 * Window controls for the custom titlebar. These are not part of the `Api` contract because
 * they talk to the webview window directly (`@tauri-apps/api/window`), not to the Rust engine.
 * They need the `core:window:allow-*` permissions listed in README.md.
 *
 * In a plain browser (mock mode) they are inert, except that maximize toggles so the
 * titlebar icon can be tried out.
 */
export interface WindowControls {
	minimize(): Promise<void>;
	toggleMaximize(): Promise<void>;
	close(): Promise<void>;
	isMaximized(): Promise<boolean>;
	/** Calls `handler` whenever the window is resized. Resolves to an unsubscribe function. */
	onResized(handler: () => void): Promise<() => void>;
}

async function tauriControls(): Promise<WindowControls> {
	const { getCurrentWindow } = await import('@tauri-apps/api/window');
	const win = getCurrentWindow();
	return {
		minimize: () => win.minimize(),
		toggleMaximize: () => win.toggleMaximize(),
		close: () => win.close(),
		isMaximized: () => win.isMaximized(),
		onResized: (handler) => win.onResized(() => handler())
	};
}

function browserControls(): WindowControls {
	let maximized = false;
	const listeners = new Set<() => void>();
	return {
		minimize: async () => {},
		toggleMaximize: async () => {
			maximized = !maximized;
			listeners.forEach((l) => l());
		},
		close: async () => {},
		isMaximized: async () => maximized,
		onResized: async (handler) => {
			listeners.add(handler);
			return () => listeners.delete(handler);
		}
	};
}

let cached: Promise<WindowControls> | null = null;

export function getWindowControls(): Promise<WindowControls> {
	cached ??= isTauri ? tauriControls() : Promise.resolve(browserControls());
	return cached;
}
