<script lang="ts">
	import { onMount } from 'svelte';
	import { invoke } from '@tauri-apps/api/core';
	import { listen } from '@tauri-apps/api/event';

	/**
	 * The message window over games. The backend shows the window and sends one message; it
	 * slides in, stays a moment and fades, then the window hides again.
	 */

	interface Message {
		title: string;
		detail: string;
		tone: 'clip' | 'recording' | 'error';
	}

	const SHOW_MS = 2600;

	let message = $state<Message | null>(null);
	let visible = $state(false);
	let timer: ReturnType<typeof setTimeout> | null = null;

	onMount(() => {
		document.documentElement.classList.add('overlay-window');
		const off = listen<Message>('overlay-message', (event) => {
			if (timer) clearTimeout(timer);
			message = event.payload;
			visible = false;
			// Restart the entrance even when a message is already up.
			requestAnimationFrame(() => (visible = true));
			timer = setTimeout(() => {
				visible = false;
				timer = setTimeout(() => void invoke('hide_overlay'), 220);
			}, SHOW_MS);
		});
		return () => {
			void off.then((f) => f());
		};
	});
</script>

{#if message}
	<div class="toast" class:visible role="status" aria-live="polite">
		<span class="mark {message.tone}" aria-hidden="true"></span>
		<span class="text">
			<span class="title">{message.title}</span>
			{#if message.detail}<span class="detail">{message.detail}</span>{/if}
		</span>
		<svg class="logo" viewBox="0 0 16 16" aria-hidden="true">
			<circle cx="8" cy="8" r="6.5" fill="none" stroke="currentColor" stroke-width="1.5" />
			<path d="M8 1.5a6.5 6.5 0 0 0 0 13z" fill="currentColor" />
		</svg>
	</div>
{/if}

<style>
	:global(html.overlay-window),
	:global(html.overlay-window body),
	:global(html.overlay-window #app) {
		background: transparent !important;
		overflow: hidden;
	}
	.toast {
		position: fixed;
		inset: 6px;
		display: flex;
		align-items: center;
		gap: 12px;
		padding: 0 16px;
		border-radius: 10px;
		border: 1px solid rgb(255 255 255 / 0.1);
		background: rgb(12 12 12 / 0.92);
		color: #ededed;
		box-shadow: 0 6px 18px rgb(0 0 0 / 0.45);
		opacity: 0;
		transform: translateY(-8px) scale(0.98);
		transition:
			opacity 180ms cubic-bezier(0.23, 1, 0.32, 1),
			transform 220ms cubic-bezier(0.23, 1, 0.32, 1);
	}
	.toast.visible {
		opacity: 1;
		transform: none;
	}
	.mark {
		width: 10px;
		height: 10px;
		flex-shrink: 0;
		border-radius: 999px;
	}
	.mark.clip {
		background: #4ade80;
	}
	.mark.recording {
		background: #dc2626;
		box-shadow: 0 0 0 4px rgb(220 38 38 / 0.25);
	}
	.mark.error {
		background: #f87171;
		border-radius: 2px;
	}
	.text {
		display: flex;
		min-width: 0;
		flex: 1;
		flex-direction: column;
		gap: 2px;
	}
	.title {
		font-size: 14px;
		font-weight: 600;
	}
	.detail {
		overflow: hidden;
		font-size: 12px;
		color: #a3a3a3;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.logo {
		width: 16px;
		height: 16px;
		flex-shrink: 0;
		color: #8c8c8c;
	}
	@media (prefers-reduced-motion: reduce) {
		.toast {
			transform: none;
			transition: opacity 120ms linear;
		}
	}
</style>
