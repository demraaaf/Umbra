import type { Api } from './types';

/**
 * True inside the Tauri webview. Everything else (`npm run dev` in a browser, the built-in
 * preview) runs against the in-memory mock, so the UI is fully usable without the Rust side.
 */
export const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
export const isMock = !isTauri;

// Only the implementation that is needed gets loaded; the mock never runs inside Tauri.
const implementation: Promise<Api> = isTauri
	? import('./tauri').then((m) => m.createTauriApi())
	: import('./mock').then((m) => m.createMockApi());

/**
 * The single backend surface for the UI. Every method forwards to the chosen implementation,
 * so callers can import `api` synchronously and never care which one is behind it.
 */
export const api: Api = new Proxy({} as Api, {
	get(_target, property) {
		if (typeof property !== 'string' || property === 'then') return undefined;
		return (...args: unknown[]) =>
			implementation.then((impl) =>
				(impl as unknown as Record<string, (...a: unknown[]) => unknown>)[property](...args)
			);
	}
});

/** Turns whatever a rejected command carries into a sentence for the user. */
export function errorMessage(error: unknown): string {
	if (typeof error === 'string') return error;
	if (error instanceof Error) return error.message;
	if (error && typeof error === 'object') {
		const candidate = (error as { message?: unknown }).message;
		if (typeof candidate === 'string') return candidate;
		try {
			return JSON.stringify(error);
		} catch {
			/* fall through */
		}
	}
	return 'Something went wrong.';
}
