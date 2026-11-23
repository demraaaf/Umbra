<script lang="ts">
	import { Toaster as Sonner, type ToasterProps as SonnerProps } from 'svelte-sonner';
	import Loader2Icon from '@lucide/svelte/icons/loader-2';
	import CheckIcon from '@lucide/svelte/icons/check';
	import XIcon from '@lucide/svelte/icons/x';
	import InfoIcon from '@lucide/svelte/icons/info';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';

	let { ...restProps }: SonnerProps = $props();
</script>

<!--
	Umbra is dark only, so there is no mode switching here. The toast surface is styled in
	app.css ([data-sonner-toast]); the confirmation tick is the only coloured element.
-->
<Sonner
	theme="dark"
	class="toaster group"
	position="bottom-right"
	offset={{ right: 16, bottom: 16 }}
	gap={8}
	visibleToasts={3}
	{...restProps}
>
	{#snippet loadingIcon()}
		<Loader2Icon class="size-4 animate-spin" />
	{/snippet}
	{#snippet successIcon()}
		<CheckIcon class="size-4 text-success" strokeWidth={2.5} />
	{/snippet}
	{#snippet errorIcon()}
		<XIcon class="size-4 text-danger" strokeWidth={2.5} />
	{/snippet}
	{#snippet infoIcon()}
		<InfoIcon class="size-4" />
	{/snippet}
	{#snippet warningIcon()}
		<TriangleAlertIcon class="size-4" />
	{/snippet}
</Sonner>
