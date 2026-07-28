<script lang="ts">
	import dagre from "@dagrejs/dagre";
	import {
		SvelteFlow,
		Panel,
		Position,
		type Node,
		type Edge,
		type NodeTypes,
	} from "@xyflow/svelte";
	import { formatRole } from "$lib/utils.js";
	import type { AgentInfo, AgentId, AgentStatus, AgentNodeData, ChatItem } from "$lib/types";
	import AgentNode from "./agent-node.svelte";
	import FitViewButton from "./fit-view-button.svelte";

	let {
		agents,
		statuses,
		chatItems,
		selected,
		onSelect,
	}: {
		agents: AgentInfo[];
		statuses: Map<AgentId, AgentStatus>;
		chatItems: ChatItem[];
		selected: AgentId | null;
		onSelect: (id: AgentId) => void;
	} = $props();

	const nodeTypes: NodeTypes = {
		agent: AgentNode as any,
	};

	// ── Node dimensions for dagre ─────────────────────────────
	const NODE_W = 120;
	const NODE_H = 120;
	const EXPANDED_W = 300;
	const EXPANDED_H = 240;

	// ── Expanded node tracking ────────────────────────────────
	let expandedNodeId = $state<AgentId | null>(null);

	// ── Role color ────────────────────────────────────────────
	function roleColor(role: string): string {
		let hash = 0;
		for (let i = 0; i < role.length; i++) {
			hash = role.charCodeAt(i) + ((hash << 5) - hash);
		}
		const hue = ((Math.abs(hash) % 360) + 360) % 360;
		return `oklch(0.6 0.18 ${hue})`;
	}

	function statusColor(s: AgentStatus): string {
		switch (s) {
			case "thinking": case "running-tool": return "#f59e0b";
			case "responding": return "#22c55e";
			case "error": return "#ef4444";
			default: return "#6b7280";
		}
	}

	// ── Build nodes and edges from agents ─────────────────────
	function buildFlow() {
		const sfNodes: Node[] = agents.map((a) => {
			const st = statuses.get(a.id) ?? "idle";
			const expanded = a.id === expandedNodeId;
			const nodeData: AgentNodeData = {
				id: a.id,
				role: a.role,
				task: a.current_task,
				status: st,
				roleColor: roleColor(a.role),
				expanded,
			};
			// Attach recent chat items only for the expanded node
			if (expanded && chatItems.length > 0) {
				nodeData.chatItems = chatItems.slice(-4);
			}
			return {
				id: String(a.id),
				type: "agent",
				position: { x: 0, y: 0 },
				data: nodeData,
				sourcePosition: Position.Right,
				targetPosition: Position.Left,
				selected: a.id === selected,
				// Raise z-index for expanded node so it renders on top
				...(expanded ? { zIndex: 100 } : {}),
			};
		});

		return { sfNodes, sfEdges: [] as Edge[] };
	}

	// ── Dagre layout ──────────────────────────────────────────
	function layoutNodes(nodes: Node[], edges: Edge[]): Node[] {
		if (nodes.length === 0) return nodes;

		const g = new dagre.graphlib.Graph();
		g.setDefaultEdgeLabel(() => ({}));
		g.setGraph({ rankdir: "LR", nodesep: 40, ranksep: 80 });

		for (const n of nodes) {
			const expanded = n.data?.expanded === true;
			g.setNode(n.id, {
				width: expanded ? EXPANDED_W : NODE_W,
				height: expanded ? EXPANDED_H : NODE_H,
			});
		}
		for (const e of edges) {
			g.setEdge(e.source, e.target);
		}

		dagre.layout(g);

		return nodes.map((n) => {
			const pos = g.node(n.id);
			const w = n.data?.expanded === true ? EXPANDED_W : NODE_W;
			const h = n.data?.expanded === true ? EXPANDED_H : NODE_H;
			return {
				...n,
				position: {
					x: pos.x - w / 2,
					y: pos.y - h / 2,
				},
			};
		});
	}

	let nodes = $state.raw<Node[]>([]);
	let edges = $state.raw<Edge[]>([]);

	// ── Single effect: rebuild on any change ──
	$effect(() => {
		agents; statuses; selected; expandedNodeId; chatItems;
		const { sfNodes } = buildFlow();
		const laid = layoutNodes(sfNodes, []);
		nodes = laid;
		edges = [];
	});

	// ── Sync: collapse expanded node when selected from outside ──
	$effect(() => {
		// When selected changes to a different agent (e.g. via sidebar),
		// collapse the currently expanded node to avoid visual inconsistency
		if (expandedNodeId && selected && selected !== expandedNodeId) {
			expandedNodeId = null;
		}
	});

	// ── Node click: toggle expansion ──────────────────────────
	function onNodeClick(event: any) {
		const id = Number(event.node?.id ?? event.target?.id);
		if (expandedNodeId === id) {
			// Clicking the already-expanded node collapses it but keeps it selected
			expandedNodeId = null;
		} else {
			expandedNodeId = id;
		}
		onSelect(id);
	}

	// ── Tooltip state ─────────────────────────────────────────
	let tooltipAgent = $state<AgentInfo | null>(null);

	function onNodePointerEnter(event: any) {
		const id = Number(event.node?.id ?? event.target?.id);
		tooltipAgent = agents.find((a) => a.id === id) ?? null;
	}

	function onNodePointerLeave() {
		tooltipAgent = null;
	}

	// ── Unique roles for legend ───────────────────────────────
	const uniqueRoles = $derived([...new Set(agents.map((a) => a.role))]);
</script>

<div class="size-full relative bg-background overflow-hidden">
	{#if agents.length === 0}
		<div class="absolute inset-0 flex items-center justify-center pointer-events-none z-10">
			<p class="text-xs text-muted-foreground/50">No agents yet</p>
		</div>
	{:else}
		<SvelteFlow
			bind:nodes
			bind:edges
			{nodeTypes}
			fitView
			fitViewOptions={{ padding: 0.3 }}
			colorMode="system"
			class="size-full"
			onnodeclick={onNodeClick}
			onnodepointerenter={onNodePointerEnter}
			onnodepointerleave={onNodePointerLeave}
			nodesDraggable={true}
			nodesFocusable={false}
			elementsSelectable={true}
			panOnDrag={true}
			zoomOnScroll={true}
		>
			<Panel position="top-right">
				<FitViewButton />
			</Panel>
		</SvelteFlow>
	{/if}

	{#if tooltipAgent && tooltipAgent.id !== expandedNodeId}
		{@const st = statuses.get(tooltipAgent.id) ?? "idle"}
		{@const sc = statusColor(st)}
		<div class="absolute top-2 left-2 flex items-center gap-2 px-2.5 py-1.5 rounded-md bg-background/90 border border-border text-xs shadow-sm backdrop-blur-sm z-20">
			<div class="size-2 rounded-full" style="background: {sc}"></div>
			<span class="font-medium">#{tooltipAgent.id} {tooltipAgent.role}</span>
			<span class="text-muted-foreground/60 text-[10px] capitalize">{st.replace("-", " ")}</span>
		</div>
	{/if}

	{#if uniqueRoles.length > 1}
		<div class="absolute bottom-12 left-2 right-2 flex flex-wrap gap-x-3 gap-y-1 justify-center pointer-events-none z-10">
			{#each uniqueRoles as role}
				<div class="flex items-center gap-1.5">
					<div class="size-2 rounded-full" style="background: {roleColor(role)}"></div>
					<span class="text-[10px] text-muted-foreground/60">{formatRole(role)}</span>
				</div>
			{/each}
		</div>
	{/if}
</div>

<style>
	/* Ensure SvelteFlow fills the container */
	:global(.svelte-flow__container) {
		width: 100% !important;
		height: 100% !important;
	}

</style>
