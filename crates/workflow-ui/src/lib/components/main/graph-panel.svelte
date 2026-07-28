<script lang="ts">
	import { formatRole } from "$lib/utils.js";
	import AgentGraph from "$lib/components/agent/agent-graph.svelte";
	import ResizeHandle from "./resize-handle.svelte";
	import type { AgentId, AgentInfo, AgentStatus } from "$lib/types";

	let {
		agents,
		statuses,
		selected,
		panelWidth,
		onSelect,
		onResize,
	}: {
		agents: AgentInfo[];
		statuses: Map<AgentId, AgentStatus>;
		selected: AgentId | null;
		panelWidth: number;
		onSelect: (id: AgentId) => void;
		onResize: (deltaX: number) => void;
	} = $props();

	const agent = $derived(agents.find((a) => a.id === selected));
	const status = $derived(selected != null ? (statuses.get(selected) ?? "idle") : "idle");

	const statusColor = $derived(
		status === "thinking" || status === "running-tool"
			? "#f59e0b"
			: status === "responding"
				? "#22c55e"
				: status === "error"
					? "#ef4444"
					: "#6b7280",
	);
</script>

<div
	class="relative shrink-0 bg-background flex flex-col min-h-0"
	style="width: {panelWidth}px"
>
	<ResizeHandle onResize={onResize} />
	<AgentGraph
		{agents}
		{statuses}
		{selected}
		{onSelect}
	/>
	{#if selected != null}
		<div class="shrink-0 border-t border-border p-3 space-y-2">
			<div class="flex items-center justify-between">
				<div class="flex items-center gap-2">
					<div
						class="size-2 rounded-full"
						style="background: {statusColor}"
					></div>
					<span class="text-xs font-medium"
						>#{agent?.id} {agent ? formatRole(agent.role) : ""}</span
					>
				</div>
				<span class="text-[10px] text-muted-foreground/50 capitalize"
					>{status.replace("-", " ")}</span
				>
			</div>
			{#if agent?.current_task}
				<div
					class="text-xs text-muted-foreground/70 bg-muted/50 rounded px-2 py-1.5 truncate"
					title={agent.current_task}
				>
					{agent.current_task}
				</div>
			{/if}
		</div>
	{/if}
</div>
