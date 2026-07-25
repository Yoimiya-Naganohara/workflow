<script lang="ts">
	import { ChevronRight } from "@lucide/svelte";

	let { text }: { text: string } = $props();
	let open = $state(false);
</script>

<div>
	<button
		onclick={() => (open = !open)}
		class="flex items-center gap-1.5 text-xs text-muted-foreground/50 hover:text-muted-foreground/80 transition-colors"
	>
		Reasoning
		<ChevronRight
			class="size-3 transition-transform duration-150 {open
				? 'rotate-90'
				: ''}"
		/>
	</button>
	{#if open}
		<div
			class="mt-1 text-xs text-muted-foreground/60 leading-relaxed whitespace-pre-wrap"
		>
			{text}
		</div>
	{/if}
</div>
