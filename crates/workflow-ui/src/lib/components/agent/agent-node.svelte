<script lang="ts">
	import { Handle, Position, type NodeProps, type Node } from "@xyflow/svelte";
	import { formatRole } from "$lib/utils.js";
	import type { AgentNodeData } from "$lib/types";

	type AgentNode = Node<AgentNodeData, string>;
	let { data }: NodeProps<AgentNode> = $props();

	const statusColor = $derived.by(() => {
		switch (data.status) {
			case "thinking": case "running-tool": return "#f59e0b";
			case "responding": return "#22c55e";
			case "error": return "#ef4444";
			default: return "#6b7280";
		}
	});

	const isActive = $derived(data.status === "thinking" || data.status === "running-tool" || data.status === "responding");
	const showPulse = $derived(data.status === "thinking" || data.status === "running-tool");
	const circumference = 2 * Math.PI * 24;
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="agent-node relative flex flex-col items-center select-none" style="width: 100px; height: 100px;">
	<Handle
		type="target"
		position={Position.Left}
		class="!bg-transparent !border-0 !size-0"
		style="top: 50%;"
	/>
	<Handle
		type="source"
		position={Position.Right}
		class="!bg-transparent !border-0 !size-0"
		style="top: 50%;"
	/>

	<!-- SVG overlay for visual elements -->
	<svg class="absolute inset-0 size-full pointer-events-none" viewBox="0 0 100 100" style="overflow: visible;">
		<defs>
			<filter id="g-glow-{data.id}" x="-50%" y="-50%" width="200%" height="200%">
				<feGaussianBlur stdDeviation="4" result="b"/>
				<feMerge>
					<feMergeNode in="b"/>
					<feMergeNode in="SourceGraphic"/>
				</feMerge>
			</filter>
		</defs>

		<!-- Glow ring -->
		<circle
			class="glow-ring"
			cx="50" cy="42" r="26"
			fill="none"
			stroke={data.roleColor}
			stroke-width="2"
			opacity="0.08"
			filter="url(#g-glow-{data.id})"
		/>

		<!-- Node body -->
		<circle
			class="node-body"
			cx="50" cy="42" r="20"
			fill={data.roleColor}
			fill-opacity="0.85"
			stroke={data.roleColor}
			stroke-width="1.5"
			stroke-opacity="0.35"
		/>

		<!-- Status ring (animated spinner via SVG animateTransform) -->
		<circle
			cx="50" cy="42" r="24"
			fill="none"
			stroke={isActive ? statusColor : "none"}
			stroke-width="2.5"
			stroke-opacity={data.status === "error" ? 0.7 : isActive ? 0.4 : 0}
			stroke-linecap="round"
			stroke-dasharray={data.status === "error" ? `${circumference} ${circumference}` : `${circumference * 0.35} ${circumference}`}
		>
			{#if data.status !== "idle" && data.status !== "error"}
				<animateTransform
					attributeName="transform"
					type="rotate"
					from="0 50 42"
					to="360 50 42"
					dur="2s"
					repeatCount="indefinite"
				/>
			{/if}
		</circle>

		<!-- Pulse ring (expanding) -->
		{#if showPulse}
			<circle
				cx="50" cy="42" r="25"
				fill="none"
				stroke={data.roleColor}
				stroke-width="2"
				opacity="0.5"
			>
				<animate attributeName="r" values="25;32;25" dur="1.8s" repeatCount="indefinite"/>
				<animate attributeName="opacity" values="0.5;0;0.5" dur="1.8s" repeatCount="indefinite"/>
			</circle>
		{/if}

		<!-- Node ID -->
		<text
			x="50" y="44"
			text-anchor="middle"
			fill="white"
			font-size="11"
			font-family="monospace"
			font-weight="700"
		>#{data.id}</text>
	</svg>

	<!-- Role label below the SVG -->
	<span class="text-[9px] text-muted-foreground/50 text-center mt-[68px] leading-tight">
		{formatRole(data.role)}
	</span>

	<!-- Task label -->
	{#if data.task}
		<span class="text-[7px] text-muted-foreground/35 text-center leading-tight max-w-[90px] truncate">
			{data.task.length > 28 ? data.task.slice(0, 28) + "…" : data.task}
		</span>
	{/if}
</div>

<style>
	/* Selected node: brighter body stroke + glow */
	:global(.svelte-flow__node.selected) .agent-node .node-body {
		stroke-width: 2.5;
		stroke-opacity: 1;
	}
	:global(.svelte-flow__node.selected) .agent-node .glow-ring {
		opacity: 0.2 !important;
	}
</style>
