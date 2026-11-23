import './app.css';
import { mount } from 'svelte';
import App from './App.svelte';
import Overlay from './Overlay.svelte';
import StatusBadge from './StatusBadge.svelte';
import { isTauri } from '$lib/api';

// A shipped desktop app should not show the webview's own context menu, but form fields keep
// theirs (copy, paste) and `npm run dev` keeps everything for debugging.
if (isTauri && import.meta.env.PROD) {
	window.addEventListener('contextmenu', (event) => {
		const target = event.target as HTMLElement | null;
		if (!target?.closest('input, textarea, .selectable')) event.preventDefault();
	});
}

// The middle mouse button selects clips (click toggles, hold and move paints a selection), so the
// webview's own middle-click behaviour is switched off everywhere: no autoscroll, no auxclick
// navigation. Capture phase, so it runs before anything else sees the event.
window.addEventListener(
	'mousedown',
	(event) => {
		if (event.button === 1) event.preventDefault();
	},
	{ capture: true }
);
window.addEventListener(
	'auxclick',
	(event) => {
		if (event.button === 1) event.preventDefault();
	},
	{ capture: true }
);

// The same page also runs the small windows over games: messages (`?overlay`) and the
// status badge (`?status`).
const query = new URLSearchParams(location.search);
const page = query.has('overlay') ? Overlay : query.has('status') ? StatusBadge : App;
const app = mount(page, { target: document.getElementById('app')! });

export default app;
