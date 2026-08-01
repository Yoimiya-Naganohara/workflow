<script lang="ts">
	import { Handle, Position, type NodeProps, type Node } from "@xyflow/svelte";
	import { formatRole } from "$lib/utils.js";
	import type { AgentNodeData } from "$lib/types";
	import TextBlock from "$lib/components/chat/text-block.svelte";
	import ThinkingBlock from "$lib/components/chat/thinking-block.svelte";
	import ToolCard from "$lib/components/chat/tool-card.svelte";
	import ErrorBlock from "$lib/components/chat/error-block.svelte";

	type AgentNode = Node<AgentNodeData, string>;
	let { data }: NodeProps<AgentNode> = $props();

	let scrollEl = $state<HTMLDivElement | null>(null);

	$effect(() => {
		data.chatItems?.length;
		if (!scrollEl || !data.expanded) return;
		requestAnimationFrame(() => {
			scrollEl!.scrollTop = scrollEl!.scrollHeight;
		});
	});

	const statusColor = $derived.by(() => {
		switch (data.status) {
			case "thinking": case "running-tool": return "#f59e0b";
			case "responding": return "#22c55e";
			case "error": return "#ef4444";
			default: return "#6b7280";
		}
	});

	const isActive = $derived(data.status === "thinking" || data.status === "running-tool" || data.status === "responding");
	const circumference = 2 * Math.PI * 24;
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
	class="agent-node relative select-none"
	style="width: {data.expanded ? 300 : 100}px; height: {data.expanded ? 260 : 100}px;"
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

	<!-- ─── COLLAPSED VIEW ─── -->
	{#if !data.expanded}
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
			<circle class="glow-ring" cx="50" cy="42" r="26" fill="none" stroke={data.roleColor} stroke-width="2" opacity="0.08" filter="url(#g-glow-{data.id})" />
			<circle class="node-body" cx="50" cy="42" r="20" fill={data.roleColor} fill-opacity="0.85" stroke={data.roleColor} stroke-width="1.5" stroke-opacity="0.35" />
			<circle cx="50" cy="42" r="24" fill="none" stroke={isActive ? statusColor : "none"} stroke-width="2.5" stroke-opacity={data.status === "error" ? 0.7 : isActive ? 0.4 : 0} stroke-linecap="round" stroke-dasharray={data.status === "error" ? `${circumference} ${circumference}` : `${circumference * 0.35} ${circumference}`}>
				{#if data.status !== "idle" && data.status !== "error"}
					<animateTransform attributeName="transform" type="rotate" from="0 50 42" to="360 50 42" dur="2s" repeatCount="indefinite" />
				{/if}
			</circle>
			<text x="50" y="44" text-anchor="middle" fill="white" font-size="11" font-family="monospace" font-weight="700">#{data.id}</text>
		</svg>
		<span class="text-[9px] text-muted-foreground/50 text-center mt-[68px] block leading-tight">{formatRole(data.role)}</span>
	{/if}

	<!-- ─── EXPANDED VIEW: chat components ─── -->
	{#if data.expanded}
		<div class="chat-card size-full rounded-xl border border-border/60 shadow-xl flex flex-col overflow-hidden bg-background/95">
			<!-- Header -->
			<div class="flex items-center gap-2 px-3 py-2 shrink-0 border-b border-border/10 bg-muted/20">
				<div class="size-2.5 rounded-full" style="background: {statusColor}"></div>
				<span class="text-xs font-semibold text-foreground/80">#{data.id} {formatRole(data.role)}</span>
				<span class="ml-auto text-[9px] text-muted-foreground/40 capitalize">{data.status.replace("-", " ")}</span>
			</div>

			<!-- Messages -->
			{#if data.chatItems && data.chatItems.length > 0}
				<div bind:this={scrollEl} class="flex-1 overflow-y-auto px-2.5 py-3 scroll-smooth edge-fade">
					<div class="space-y-1.5">
						{#each data.chatItems as item}
							<div class="scroll-line">
								{#if item.type === "text"}
									<TextBlock text={item.text} role="assistant" streaming={true} />
								{:else if item.type === "user"}
									<TextBlock text={item.text} role="user" />
								{:else if item.type === "thinking"}
									<ThinkingBlock text={item.text} />
								{:else if item.type === "tool"}
									<ToolCard name={item.text} result={item.result ?? undefined} status={item.status ?? "done"} />
								{:else if item.type === "error"}
									<ErrorBlock text={item.text} />
								{/if}
							</div>
						{/each}
					</div>
				</div>
			{:else}
				<div class="flex-1 flex items-center justify-center p-3">
					<span class="text-xs text-muted-foreground/30">No messages yet</span>
				</div>
			{/if}
		</div>
	{/if}
</div>

<style>
	/* ── Selected node: brighter body stroke + glow ── */
	:global(.svelte-flow__node.selected) .agent-node:not(.expanded) .node-body {
		stroke-width: 2.5;
		stroke-opacity: 1;
	}
	:global(.svelte-flow__node.selected) .agent-node:not(.expanded) .glow-ring {
		opacity: 0.2 !important;
	}

	/* ── Hover lift for collapsed nodes ── */
	:global(.svelte-flow__node:not(.dragging)) .agent-node:not(.expanded) {
		transition: width 0.35s cubic-bezier(0.34, 1.56, 0.64, 1),
		            height 0.35s cubic-bezier(0.34, 1.56, 0.64, 1),
		            transform 0.2s ease,
		            filter 0.2s ease;
	}
	:global(.svelte-flow__node:not(.dragging)) .agent-node:not(.expanded):hover {
		transform: scale(1.08);
		filter: brightness(1.15);
	}

	/* ── Base size transition ── */
	.agent-node {
		transition: width 0.35s cubic-bezier(0.34, 1.56, 0.64, 1),
		            height 0.35s cubic-bezier(0.34, 1.56, 0.64, 1);
	}

	/* ── Chat card entrance ── */
	.chat-card {
		animation: chat-appear 0.3s cubic-bezier(0.34, 1.56, 0.64, 1) forwards;
	}
	@keyframes chat-appear {
		from { opacity: 0; transform: scale(0.95); }
		to   { opacity: 1; transform: scale(1); }
	}

	/* ── Message scroll-in ── */
	.scroll-line {
		animation: chat-enter 0.25s ease-out both;
	}
	.scroll-line:nth-child(1) { animation-delay: 0.02s; }
	.scroll-line:nth-child(2) { animation-delay: 0.06s; }
	.scroll-line:nth-child(3) { animation-delay: 0.10s; }
	.scroll-line:nth-child(4) { animation-delay: 0.14s; }
	@keyframes chat-enter {
		from { opacity: 0; transform: translateY(8px); }
		to   { opacity: 1; transform: translateY(0); }
	}

	/* ── Edge fade mask ── */
	.edge-fade {
		-webkit-mask-image: linear-gradient(
			to bottom,
			transparent 0%,
			black 15%,
			black 85%,
			transparent 100%
		);
		mask-image: linear-gradient(
			to bottom,
			transparent 0%,
			black 15%,
			black 85%,
			transparent 100%
		);
	}

	/* ── Compact embedded chat components ── */
	.chat-card :global(.text-sm) {
		font-size: 0.75rem !important;
		line-height: 1.3 !important;
	}
	.chat-card :global(.inline-block) {
		max-width: 95% !important;
	}

	/* ── SVG status-ring opacity transition ── */
	:global(.svelte-flow__node) .agent-node svg circle[stroke-dasharray] {
		transition: opacity 0.4s ease;
	}
</style>
