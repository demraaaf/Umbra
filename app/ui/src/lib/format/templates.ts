// Live preview for the storage templates. Mirrors what the engine substitutes for the
// placeholders listed in `TEMPLATE_PLACEHOLDERS` (types.ts).

import { TEMPLATE_PLACEHOLDERS, type StorageSettings } from '$lib/api/types';

const KNOWN = new Set<string>(TEMPLATE_PLACEHOLDERS.map((p) => p.token));
const pad = (v: number) => String(v).padStart(2, '0');

export interface TemplateContext {
	game: string;
	when: Date;
}

export function tokensIn(template: string): string[] {
	return template.match(/\{[^{}]*\}/g) ?? [];
}

export function unknownTokens(template: string): string[] {
	return [...new Set(tokensIn(template).filter((t) => !KNOWN.has(t)))];
}

export function renderTemplate(template: string, ctx: TemplateContext): string {
	const { when } = ctx;
	const values: Record<string, string> = {
		'{game}': ctx.game,
		'{date}': `${when.getFullYear()}-${pad(when.getMonth() + 1)}-${pad(when.getDate())}`,
		'{time}': `${pad(when.getHours())}-${pad(when.getMinutes())}-${pad(when.getSeconds())}`,
		'{year}': String(when.getFullYear()),
		'{month}': pad(when.getMonth() + 1),
		'{day}': pad(when.getDate())
	};
	return template.replace(/\{[^{}]*\}/g, (token) => values[token] ?? token);
}

const INVALID_NAME_CHARS = /[<>:"/\\|?*\u0000-\u001f]/g;

/** The engine replaces characters Windows does not allow in a file or folder name with `_`. */
export function sanitizeSegment(segment: string): string {
	return segment.replace(INVALID_NAME_CHARS, '_').replace(/[. ]+$/g, '').trim();
}

export function hasInvalidChars(template: string): boolean {
	// Slashes are fine in the folder template (they make nested folders) but not in the name.
	return /[<>:"|?*\u0000-\u001f]/.test(template.replace(/\{[^{}]*\}/g, ''));
}

export function nameHasSeparator(template: string): boolean {
	return /[/\\]/.test(template.replace(/\{[^{}]*\}/g, ''));
}

/** Full example path for the given storage settings, using Windows separators. */
export function previewPath(storage: StorageSettings, ctx: TemplateContext): string {
	const folder = storage.folder.replace(/[\\/]+$/, '');
	const sub = renderTemplate(storage.folderTemplate, ctx)
		.split(/[\\/]+/)
		.map(sanitizeSegment)
		.filter(Boolean);
	const name = sanitizeSegment(renderTemplate(storage.nameTemplate, ctx)) || 'clip';
	return [folder, ...sub, `${name}.mp4`].join('\\');
}
