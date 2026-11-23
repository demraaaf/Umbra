// Days for grouping the library: clips are listed under Today, Yesterday, the weekday for the
// rest of the past week, then the date (with the year when it is not this year).

/** The local calendar day of an ISO time, as `2026-10-02`. */
export function dayKey(iso: string): string {
	const d = new Date(iso);
	if (Number.isNaN(d.getTime())) return '';
	return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}

export function dayLabel(iso: string, now = new Date()): string {
	const d = new Date(iso);
	if (Number.isNaN(d.getTime())) return 'Unknown date';
	const midnight = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
	const days = Math.round((midnight(now) - midnight(d)) / 86_400_000);
	if (days === 0) return 'Today';
	if (days === 1) return 'Yesterday';
	if (days > 1 && days < 7) return d.toLocaleDateString('en-GB', { weekday: 'long' });
	return d.toLocaleDateString('en-GB', {
		day: 'numeric',
		month: 'long',
		...(d.getFullYear() === now.getFullYear() ? {} : { year: 'numeric' })
	});
}

/** Splits clips (already sorted) into runs of the same day, keeping their order. */
export function byDay<T extends { createdAt: string }>(items: T[]): { key: string; label: string; items: T[] }[] {
	const groups: { key: string; label: string; items: T[] }[] = [];
	for (const item of items) {
		const key = dayKey(item.createdAt);
		const last = groups[groups.length - 1];
		if (last && last.key === key) last.items.push(item);
		else groups.push({ key, label: dayLabel(item.createdAt), items: [item] });
	}
	return groups;
}
