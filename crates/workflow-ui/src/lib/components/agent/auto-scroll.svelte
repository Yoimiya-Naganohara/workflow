<script lang="ts">
	import { tick } from "svelte";
	import { useSvelteFlow } from "@xyflow/svelte";
	import type { AgentId } from "$lib/types";

	let {
		expandedNodeId,
	}: {
		expandedNodeId: AgentId | null;
	} = $props();

	const { fitView } = useSvelteFlow();

	$effect(() => {
		const id = expandedNodeId;
		tick().then(() => {
			if (id != null) {
				fitView({
					nodes: [{ id: String(id) }],
					duration: 400,
					padding: 0.35,
				});
			} else {
				fitView({ duration: 400, padding: 0.3 });
			}
		});
	});
</script>
