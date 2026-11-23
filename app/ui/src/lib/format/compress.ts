// The qualities clips can be compressed at, by hand or in the background.

import type { ShrinkQuality } from '$lib/api/types';

export const COMPRESS_QUALITIES: { value: ShrinkQuality; label: string; detail: string }[] = [
	{ value: 'small', label: 'Smaller', detail: 'about 8 Mbit/s at 1440p60' },
	{ value: 'balanced', label: 'Balanced', detail: 'about 12 Mbit/s at 1440p60' },
	{ value: 'high', label: 'Better', detail: 'about 19 Mbit/s at 1440p60' }
];
