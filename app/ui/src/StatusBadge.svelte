<script lang="ts">
	import { onMount } from 'svelte';
	import { listen } from '@tauri-apps/api/event';

	/**
	 * The badge that stays over the game: recording with its time, paused, the replay buffer
	 * with how much it holds, or off. The backend shows, moves and hides the window and sends
	 * the state about once a second; the time counts on between updates so it ticks smoothly.
	 * Windows keeps this window out of captures, so it is never in a clip.
	 */
	interface Badge {
		mode: 'recording' | 'paused' | 'buffer' | 'off';
		secs: number;
	}

	let badge = $state<Badge | null>(null);
	let received = 0;
	let now = $state(performance.now());

	onMount(() => {
		document.documentElement.classList.add('overlay-window');
		const off = listen<Badge>('status-badge', (event) => {
			badge = event.payload;
			received = performance.now();
		});
		const timer = setInterval(() => (now = performance.now()), 250);
		return () => {
			clearInterval(timer);
			void off.then((f) => f());
		};
	});

	/** A recording's time keeps counting between updates; a buffer's length does not. */
	const secs = $derived(badge ? badge.secs + (badge.mode === 'recording' ? Math.max(0, now - received) / 1000 : 0) : 0);

	function clock(total: number): string {
		const s = Math.max(0, Math.floor(total));
		const h = Math.floor(s / 3600);
		const m = Math.floor((s % 3600) / 60);
		const pad = (n: number) => String(n).padStart(2, '0');
		return h > 0 ? `${h}:${pad(m)}:${pad(s % 60)}` : `${m}:${pad(s % 60)}`;
	}

	const label = $derived(
		!badge ? '' : badge.mode === 'recording' ? 'REC' : badge.mode === 'paused' ? 'Paused' : badge.mode === 'buffer' ? 'Replay' : 'Off'
	);
</script>

{#if badge}
	<div class="badge {badge.mode}" role="status">
		<span class="dot" aria-hidden="true"></span>
		<span class="label">{label}</span>
		{#if badge.mode !== 'off'}
			<span class="time">{clock(secs)}</span>
		{/if}
	</div>
{/if}

<style>
	:global(html.overlay-window),
	:global(html.overlay-window body),
	:global(html.overlay-window #app) {
		background: transparent !important;
		overflow: hidden;
	}
	.badge {
		position: fixed;
		top: 6px;
		left: 6px;
		display: inline-flex;
		height: 30px;
		align-items: center;
		gap: 8px;
		padding: 0 12px 0 10px;
		border-radius: 999px;
		border: 1px solid rgb(255 255 255 / 0.1);
		background: rgb(12 12 12 / 0.78);
		color: #ededed;
		font:
			600 12px/1 Inter,
			system-ui,
			sans-serif;
		letter-spacing: 0.02em;
	}
	.dot {
		width: 8px;
		height: 8px;
		border-radius: 999px;
		background: #737373;
	}
	.recording .dot {
		background: #dc2626;
		animation: pulse 1.6s ease-in-out infinite;
	}
	.paused .dot {
		background: #f59e0b;
	}
	.buffer .dot {
		background: #4ade80;
	}
	.off {
		color: #a3a3a3;
	}
	.time {
		font-variant-numeric: tabular-nums;
		font-weight: 500;
		color: #d4d4d4;
	}
	@keyframes pulse {
		50% {
			opacity: 0.35;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.recording .dot {
			animation: none;
		}
	}
</style>
