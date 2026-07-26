<script lang="ts">
	import { useSvelteFlow } from "@xyflow/svelte";
	import { Button } from "$lib/components/ui/button";

	const { fitView } = useSvelteFlow();
</script>

<Button variant="ghost" size="icon-xs" onclick={() => fitView()} title="Fit to view">
	<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-3.5"><path d="M15 3h6v6"/><path d="M9 21H3v-6"/><path d="M21 3l-7 7"/><path d="M3 21l7-7"/></svg>
</Button>
