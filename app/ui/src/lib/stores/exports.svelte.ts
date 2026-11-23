// Exports that are running, so their progress shows at the bottom of the sidebar on any screen.

export interface RunningExport {
	/** The clip being exported; the editor keeps its session. */
	path: string;
	name: string;
	fraction: number;
	/** `performance.now()` when it began. */
	started: number;
}

export const exports = $state({ running: [] as RunningExport[] });

export function beginExport(path: string, name: string) {
	exports.running = [...exports.running.filter((e) => e.path !== path), { path, name, fraction: 0, started: performance.now() }];
}

export function updateExport(path: string, fraction: number) {
	const item = exports.running.find((e) => e.path === path);
	if (item) item.fraction = fraction;
}

export function endExport(path: string) {
	exports.running = exports.running.filter((e) => e.path !== path);
}

/** Seconds left at the pace so far, once there is enough of it to tell. */
export function exportSecsLeft(item: RunningExport, now = performance.now()): number | null {
	const elapsed = (now - item.started) / 1000;
	if (item.fraction < 0.01 || elapsed < 3) return null;
	return (elapsed * (1 - item.fraction)) / item.fraction;
}
