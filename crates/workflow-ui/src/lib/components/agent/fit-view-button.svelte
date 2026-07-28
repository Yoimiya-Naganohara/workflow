<script lang="ts">
		import { useSvelteFlow } from "@xyflow/svelte";
		import { Button } from "$lib/components/ui/button";
		import { Maximize2 } from "@lucide/svelte";

		const { fitView } = useSvelteFlow();
	</script>

	<Button variant="ghost" size="icon-xs" onclick={() => fitView()} title="Fit to view">
		<Maximize2 class="size-3.5" />
	</Button>
