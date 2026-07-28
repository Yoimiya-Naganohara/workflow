<script lang="ts">
	import { Handle, Position, type NodeProps, type Node } from "@xyflow/svelte";
	import { formatRole } from "$lib/utils.js";
	import type { AgentNodeData } from "$lib/types";

	type AgentNode = Node<AgentNodeData, string>;
	let { data }: NodeProps<AgentNode> = $props();

	let scrollEl = $state<HTMLDivElement | null>(null);

	// Auto-scroll to bottom when messages change
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

	function messageIcon(type: string): string {
		switch (type) {
			case "user": return "💬";
			case "assistant": return "🤖";
			case "thinking": return "💭";
			case "tool": return "🔧";
			case "error": return "⚠️";
			default: return "💬";
		}
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

	<!-- ─── EXPANDED VIEW: terminal scrolling ─── -->
	{#if data.expanded}
		<div
			class="terminal-card size-full rounded-xl border border-border/60 shadow-xl flex flex-col overflow-hidden"
			style="border-top: 3px solid {data.roleColor};"
		>
			<!-- Terminal header bar -->
			<div
				class="flex items-center gap-1.5 px-3 py-2 shrink-0 border-b border-border/20 bg-muted/30"
			>
				<div class="size-2 rounded-full" style="background: {statusColor}"></div>
				<span class="text-[11px] font-mono font-bold text-foreground/80">
					#{data.id} {formatRole(data.role)}
				</span>
				<span class="ml-auto text-[9px] font-mono text-muted-foreground/40">
					{data.status.replace("-", " ")}
				</span>
			</div>

			<!-- Terminal messages -->
			{#if data.chatItems && data.chatItems.length > 0}
				<div
					bind:this={scrollEl}
					class="flex-1 overflow-y-auto px-2.5 py-2 font-mono text-[11px] leading-relaxed space-y-1 scroll-smooth"
				>
					{#each data.chatItems as item}
						<div class="scroll-line flex items-start gap-2">
							<span class="shrink-0 mt-0.5">{messageIcon(item.type)}</span>
							<span
								class="terminal-text leading-snug"
								class:type-thinking={item.type === "thinking"}
								class:type-assistant={item.type === "assistant"}
								class:type-error={item.type === "error"}
								class:type-tool={item.type === "tool"}
							>{(item.type === "tool" ? item.text : item.text).slice(0, 180)}{item.text.length > 180 ? "…" : ""}</span
						>
						</div>
					{/each}
				</div>
			{:else}
				<div class="flex-1 flex items-center justify-center p-3">
					<span class="text-[11px] font-mono text-muted-foreground/30 italic">
						<span class="animate-pulse">_</span>
					</span>
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

	/* ── Base size transition for .agent-node ── */
	.agent-node {
		transition: width 0.35s cubic-bezier(0.34, 1.56, 0.64, 1),
		            height 0.35s cubic-bezier(0.34, 1.56, 0.64, 1);
	}

	/* ── Terminal card entrance ── */
	.terminal-card {
		animation: terminal-appear 0.3s cubic-bezier(0.34, 1.56, 0.64, 1) forwards;
	}

	@keyframes terminal-appear {
		from {
			opacity: 0;
			transform: scale(0.95);
		}
		to {
			opacity: 1;
			transform: scale(1);
		}
	}

	/* ── Terminal scroll-in lines ── */
	.scroll-line {
		animation: scroll-up 0.35s ease-out both;
		overflow: hidden;
	}

	.scroll-line:nth-child(1) { animation-delay: 0.02s; }
	.scroll-line:nth-child(2) { animation-delay: 0.06s; }
	.scroll-line:nth-child(3) { animation-delay: 0.10s; }
	.scroll-line:nth-child(4) { animation-delay: 0.14s; }

	@keyframes scroll-up {
		from {
			opacity: 0;
			transform: translateY(12px);
			max-height: 0;
		}
		to {
			opacity: 1;
			transform: translateY(0);
			max-height: 60px;
		}
	}

	/* ── Terminal text colors (default + type-specific) ── */
	.terminal-text {
		display: inline;
		color: rgb(156 163 175 / 0.8);
	}
	.type-thinking { color: rgb(252 211 77 / 0.9); }
	.type-assistant { color: rgb(74 222 128 / 0.9); }
	.type-error    { color: rgb(248 113 113 / 0.8); }
	.type-tool     { color: rgb(125 211 252 / 0.8); }

	/* ── Terminal text scroll reveal ── */
	.terminal-text {
		animation: text-reveal 0.3s ease-out both;
	}

	.scroll-line:nth-child(1) .terminal-text { animation-delay: 0.10s; }
	.scroll-line:nth-child(2) .terminal-text { animation-delay: 0.20s; }
	.scroll-line:nth-child(3) .terminal-text { animation-delay: 0.30s; }
	.scroll-line:nth-child(4) .terminal-text { animation-delay: 0.40s; }

	@keyframes text-reveal {
		from {
			opacity: 0;
			clip-path: inset(0 100% 0 0);
		}
		to {
			opacity: 1;
			clip-path: inset(0 0 0 0);
		}
	}

	/* ── SVG status-ring opacity transition ── */
	:global(.svelte-flow__node) .agent-node svg circle[stroke-dasharray] {
		transition: opacity 0.4s ease;
	}
</style>
