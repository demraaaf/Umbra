<script lang="ts">
	import UserRoundIcon from '@lucide/svelte/icons/user-round';
</script>

<!-- Where the camera goes, when its picture can't be shown here. -->
<div class="stand-in flex size-full items-center justify-center" aria-hidden="true">
	<UserRoundIcon class="size-1/2 max-h-24 max-w-24 text-subtle" strokeWidth={1.25} />
</div>

<style>
	.stand-in {
		background: linear-gradient(160deg, #2a2a2a, #161616);
	}
</style>
