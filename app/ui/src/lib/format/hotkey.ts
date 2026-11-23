// Hotkey strings use the format documented on `ClipSettings.hotkey` in types.ts:
// modifiers (`Ctrl`, `Alt`, `Shift`, `Win`) then one key, joined by `+`. The key is a
// `KeyboardEvent.code` value or a mouse button (`Mouse3` middle, `Mouse4` back, `Mouse5` forward).

const MODIFIER_ORDER = ['Ctrl', 'Alt', 'Shift', 'Win'] as const;
type Modifier = (typeof MODIFIER_ORDER)[number];

const MODIFIER_CODES: Record<string, Modifier> = {
	ControlLeft: 'Ctrl',
	ControlRight: 'Ctrl',
	AltLeft: 'Alt',
	AltRight: 'Alt',
	ShiftLeft: 'Shift',
	ShiftRight: 'Shift',
	MetaLeft: 'Win',
	MetaRight: 'Win'
};

export function isModifierCode(code: string): boolean {
	return code in MODIFIER_CODES;
}

function modifiersFrom(e: { ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean }): Modifier[] {
	const active: Modifier[] = [];
	if (e.ctrlKey) active.push('Ctrl');
	if (e.altKey) active.push('Alt');
	if (e.shiftKey) active.push('Shift');
	if (e.metaKey) active.push('Win');
	return active;
}

/** Builds `Alt+F10` from a keydown. Returns null while only modifiers are held. */
export function comboFromKeyboardEvent(e: KeyboardEvent): string | null {
	if (!e.code || isModifierCode(e.code)) return null;
	return [...modifiersFrom(e), e.code].join('+');
}

/** Builds `Mouse4` (or `Alt+Mouse4`) from a mouse button. Left and right buttons are not allowed. */
export function comboFromMouseEvent(e: MouseEvent): string | null {
	const names: Record<number, string> = { 1: 'Mouse3', 3: 'Mouse4', 4: 'Mouse5' };
	const key = names[e.button];
	return key ? [...modifiersFrom(e), key].join('+') : null;
}

export function splitCombo(combo: string): string[] {
	return combo.split('+').filter(Boolean);
}

const KEY_LABELS: Record<string, string> = {
	Escape: 'Esc',
	Backquote: '`',
	Minus: '-',
	Equal: '=',
	BracketLeft: '[',
	BracketRight: ']',
	Backslash: '\\',
	Semicolon: ';',
	Quote: "'",
	Comma: ',',
	Period: '.',
	Slash: '/',
	Space: 'Space',
	Enter: 'Enter',
	Backspace: 'Backspace',
	Tab: 'Tab',
	Delete: 'Del',
	Insert: 'Ins',
	PageUp: 'PgUp',
	PageDown: 'PgDn',
	Home: 'Home',
	End: 'End',
	ArrowUp: 'Up',
	ArrowDown: 'Down',
	ArrowLeft: 'Left',
	ArrowRight: 'Right',
	CapsLock: 'Caps',
	NumpadAdd: 'Num +',
	NumpadSubtract: 'Num -',
	NumpadMultiply: 'Num *',
	NumpadDivide: 'Num /',
	NumpadDecimal: 'Num .',
	NumpadEnter: 'Num Enter',
	PrintScreen: 'PrtSc',
	ScrollLock: 'ScrLk',
	Pause: 'Pause',
	Mouse3: 'Mouse 3',
	Mouse4: 'Mouse 4',
	Mouse5: 'Mouse 5'
};

/** A short label for one part of a combo: `KeyG` becomes `G`, `Numpad5` becomes `Num 5`. */
export function keyLabel(part: string): string {
	if (KEY_LABELS[part]) return KEY_LABELS[part];
	if (/^Key[A-Z]$/.test(part)) return part.slice(3);
	if (/^Digit\d$/.test(part)) return part.slice(5);
	if (/^Numpad\d$/.test(part)) return `Num ${part.slice(6)}`;
	return part;
}

/** The bindings of a hotkey setting: `Mouse4, Mouse5` holds two, each triggering the action. */
export function bindingsOf(value: string): string[] {
	return value
		.split(',')
		.map((b) => b.trim())
		.filter(Boolean);
}

/** Labels of the first binding, for short hints on buttons. */
export function comboLabels(value: string): string[] {
	return splitCombo(bindingsOf(value)[0] ?? '').map(keyLabel);
}

/** Labels of every binding, for lists of hotkeys. */
export function bindingLabels(value: string): string[][] {
	return bindingsOf(value).map((b) => splitCombo(b).map(keyLabel));
}

export function isValidCombo(combo: string): boolean {
	const parts = splitCombo(combo);
	if (parts.length === 0) return false;
	const key = parts[parts.length - 1];
	const mods = parts.slice(0, -1);
	return !isModifierCode(key) && mods.every((m) => (MODIFIER_ORDER as readonly string[]).includes(m));
}

/** True for a bare letter, digit or similar key that would also type while in a game. */
export function isTypingKey(value: string): boolean {
	return bindingsOf(value).some(isTypingCombo);
}

function isTypingCombo(combo: string): boolean {
	const parts = splitCombo(combo);
	if (parts.length !== 1) return false;
	return /^(Key[A-Z]|Digit\d|Space|Enter|Comma|Period|Slash|Semicolon|Quote|Minus|Equal|Backquote)$/.test(parts[0]);
}
