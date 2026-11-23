import type { BurnLayers } from '$lib/api/types';

/** The Burn in choices for clips with `has`: each layer they have, and all of them when there are several. */
export function burnChoices(has: BurnLayers): { label: string; layers: BurnLayers }[] {
	const none: BurnLayers = { camera: false, input: false, logo: false };
	const list: { label: string; layers: BurnLayers }[] = [];
	if (has.camera) list.push({ label: 'Camera', layers: { ...none, camera: true } });
	if (has.input) list.push({ label: 'Input overlay', layers: { ...none, input: true } });
	if (has.logo) list.push({ label: 'Logo', layers: { ...none, logo: true } });
	if (list.length > 1) list.push({ label: 'All layers', layers: { ...has } });
	return list;
}
