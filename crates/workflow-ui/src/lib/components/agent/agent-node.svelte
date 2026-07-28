<script lang="ts">
	import { Handle, Position, type NodeProps, type Node } from "@xyflow/svelte";
	import { formatRole } from "$lib/utils.js";
	import type { AgentNodeData, ChatItem } from "$lib/types";

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

	function messageIcon(type: ChatItem["type"]): string {
		switch (type) {
			case "user": return "💬";
			case "assistant": return "🤖";
			case "thinking": return "💭";
			case "tool": return "🔧";
			case "error": return "⚠️";
		}
	}

	function truncate(text: string, max: number): string {
		return text.length > max ? text.slice(0, max) + "…" : text;
	}
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
	class="agent-node relative select-none"
	style="width: {data.expanded ? 280 : 100}px; height: {data.expanded ? 220 : 100}px;"
>
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

	<!-- ─── COLLAPSED VIEW: existing circular node ─── -->
	{#if !data.expanded}
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
		<span class="text-[9px] text-muted-foreground/50 text-center mt-[68px] block leading-tight">
			{formatRole(data.role)}
		</span>

		<!-- Task label -->
		{#if data.task}
			<span class="text-[7px] text-muted-foreground/35 text-center leading-tight block max-w-[90px] truncate mx-auto">
				{data.task.length > 28 ? data.task.slice(0, 28) + "…" : data.task}
			</span>
		{/if}
	{/if}

	<!-- ─── EXPANDED VIEW: agent info card ─── -->
	{#if data.expanded}
		<div
			class="expanded-card size-full rounded-xl border border-border/60 bg-background/90 backdrop-blur-md shadow-xl flex flex-col overflow-hidden"
			style="border-top: 3px solid {data.roleColor};"
		>
			<!-- Header -->
			<div class="flex items-center gap-2 px-3 pt-2.5 pb-1.5">
				<div class="size-3 rounded-full shrink-0" style="background: {statusColor}"></div>
				<span class="text-sm font-bold text-foreground leading-tight">
					#{data.id} {formatRole(data.role)}
				</span>
				<div class="ml-auto flex items-center gap-1.5">
					{#if showPulse}
						<span class="size-1.5 rounded-full bg-amber-400 animate-pulse"></span>
					{/if}
					<span
						class="text-[10px] capitalize px-1.5 py-0.5 rounded-full bg-muted text-muted-foreground/70 font-medium leading-none"
					>
						{data.status.replace("-", " ")}
					</span>
				</div>
			</div>

			<!-- Task (if present) -->
			{#if data.task}
				<div class="px-3 pb-1.5">
					<div
						class="text-[11px] text-muted-foreground/80 bg-muted/40 rounded-md px-2 py-1.5 leading-relaxed line-clamp-2 border border-border/30"
						title={data.task}
					>
						<span class="font-medium text-muted-foreground/60 mr-1">📋</span>
						{data.task}
					</div>
				</div>
			{/if}

			<!-- Recent activity -->
			{#if data.chatItems && data.chatItems.length > 0}
				<div class="flex-1 flex flex-col min-h-0 px-3 pb-2.5">
					<div class="text-[9px] font-semibold text-muted-foreground/40 uppercase tracking-wider mb-1 mt-0.5">
						Recent Activity
					</div>
					<div class="flex-1 space-y-0.5 overflow-y-auto min-h-0 max-h-[108px]">
						{#each data.chatItems as item}
							<div
								class="flex items-start gap-1.5 text-[11px] px-1.5 py-1 rounded-md hover:bg-muted/30 transition-colors"
							>
								<span class="shrink-0 mt-0.5 text-[10px]">{messageIcon(item.type)}</span>
								<span class="text-muted-foreground/75 leading-snug line-clamp-2 min-w-0">
									{item.type === "tool" ? item.text : truncate(item.text, 140)}
								</span>
							</div>
						{/each}
					</div>
					<div class="text-[9px] text-muted-foreground/30 text-center pt-1 border-t border-border/20 mt-1">
						showing {data.chatItems.length} message{data.chatItems.length !== 1 ? "s" : ""}
					</div>
				</div>
			{:else}
				<div class="flex-1 flex items-center justify-center px-3 pb-2.5">
					<span class="text-[11px] text-muted-foreground/30 italic">No recent activity</span>
				</div>
			{/if}
		</div>
	{/if}
</div>

<style>
	/* Selected node: brighter body stroke + glow */
	:global(.svelte-flow__node.selected) .agent-node:not(.expanded) .node-body {
		stroke-width: 2.5;
		stroke-opacity: 1;
	}
	:global(.svelte-flow__node.selected) .agent-node:not(.expanded) .glow-ring {
		opacity: 0.2 !important;
	}

	/* Expanded card transitions */
	.agent-node {
		transition: width 0.25s ease, height 0.25s ease;
	}
</style>
