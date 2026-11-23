<script lang="ts">
	import { isMock } from '$lib/api';
	import { app } from '$lib/stores/app.svelte';
	import { CODEC_LABELS } from '$lib/format/format';
</script>

<!-- Quiet reference information, moved here from the sidebar. -->
<section id="about" class="scroll-mt-4 pt-4 pb-6" aria-labelledby="about-title" data-section="about">
	<h2 id="about-title" class="pb-2 text-base font-semibold text-foreground">About</h2>
	{#if app.systemInfo}
		<dl class="grid grid-cols-[160px_1fr] gap-x-6 gap-y-2 border-t border-line pt-3 text-xs">
			<dt class="text-muted-foreground">Version</dt>
			<dd class="tnum selectable text-foreground">Umbra {app.systemInfo.appVersion}{isMock ? ' (mock data)' : ''}</dd>
			<dt class="text-muted-foreground">Graphics card</dt>
			<dd class="selectable text-foreground">{app.systemInfo.gpu}</dd>
			<dt class="text-muted-foreground">Hardware encoding</dt>
			<dd class="text-foreground">{app.systemInfo.codecs.map((c) => CODEC_LABELS[c]).join(', ') || 'None found'}</dd>
		</dl>
	{/if}
</section>
