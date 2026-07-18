<script lang="ts">
	import { onMount } from "svelte";
	import * as d3 from "d3";
	import type { AgentInfo, AgentId, AgentStatus } from "$lib/types";

	interface GraphNode extends d3.SimulationNodeDatum {
		id: number;
		role: string;
	}

	interface GraphEdge {
		source: number;
		target: number;
	}

	let {
		agents,
		statuses,
		selected,
		onSelect,
	}: {
		agents: AgentInfo[];
		statuses: Map<AgentId, AgentStatus>;
		selected: AgentId | null;
		onSelect: (id: AgentId) => void;
	} = $props();

	let svgEl: SVGSVGElement;
	let containerEl: HTMLDivElement;
	let sim: d3.Simulation<GraphNode, undefined> | null = null;
	let ro: ResizeObserver;
	let tooltipAgent = $state<AgentInfo | null>(null);

	const uniqueRoles = $derived([...new Set(agents.map(a => a.role))]);

	const roleColors: Record<string, string> = {
		planner:  "oklch(0.6 0.18 260)",
		executor: "oklch(0.6 0.18 145)",
		reviewer: "oklch(0.65 0.18 85)",
		reporter: "oklch(0.6 0.12 195)",
		default:  "oklch(0.6 0.18 300)",
	};

	function roleColor(role: string): string {
		return roleColors[role] ?? roleColors.default;
	}

	const nodeCache = new Map<number, GraphNode>();

	function getNodes(): GraphNode[] {
		return agents.map(a => {
			let n = nodeCache.get(a.id);
			if (!n) {
				n = { id: a.id, role: a.role, x: NaN, y: NaN };
				nodeCache.set(a.id, n);
			}
			n.role = a.role;
			return n;
		});
	}

	function getEdges(nodes: GraphNode[]): GraphEdge[] {
		if (nodes.length < 2) return [];
		const edges: GraphEdge[] = [];
		for (let i = 1; i < nodes.length; i++) {
			edges.push({ source: nodes[i - 1].id, target: nodes[i].id });
		}
		return edges;
	}

	let g: d3.Selection<SVGGElement, unknown, null, undefined>;
	let nodeGroup: d3.Selection<SVGGElement, GraphNode, SVGGElement, unknown>;
	let edgeLines: d3.Selection<SVGLineElement, GraphEdge, SVGGElement, unknown>;

	function init() {
		if (!svgEl || !containerEl || agents.length === 0) return;

		if (sim) { sim.stop(); sim = null; }

		const width = containerEl.clientWidth;
		const height = containerEl.clientHeight;
		if (width === 0 || height === 0) return;

		const svg = d3.select(svgEl);
		svg.selectAll("*").remove();

		const defs = svg.append("defs");

		const gl = defs.append("filter").attr("id", "g-glow").attr("x", "-50%").attr("y", "-50%").attr("width", "200%").attr("height", "200%");
		gl.append("feGaussianBlur").attr("stdDeviation", "4").attr("result", "b");
		const gm = gl.append("feMerge");
		gm.append("feMergeNode").attr("in", "b");
		gm.append("feMergeNode").attr("in", "SourceGraphic");

		g = svg.append("g").attr("class", "graph-root");

		const zoom = d3.zoom<SVGSVGElement, unknown>()
			.extent([[0, 0], [width, height]])
			.scaleExtent([0.1, 4])
			.on("zoom", (event) => g.attr("transform", event.transform.toString()));
		svg.call(zoom);

		const nodes = getNodes();
		const edges = getEdges(nodes);

		edgeLines = g.selectAll<SVGLineElement, GraphEdge>("line")
			.data(edges)
			.join("line")
			.attr("stroke", "currentColor")
			.attr("stroke-opacity", 0.08)
			.attr("stroke-width", 1);

		nodeGroup = g.selectAll<SVGGElement, GraphNode>("g")
			.data(nodes, (d: GraphNode) => String(d.id))
			.join("g")
			.attr("data-id", (d: GraphNode) => d.id)
			.attr("class", "agent-node cursor-pointer")
			.on("click", (_event: any, d: GraphNode) => { onSelect(d.id); })
			.on("mouseenter", (_event: any, d: GraphNode) => {
				tooltipAgent = agents.find(a => a.id === d.id) ?? null;
			})
			.on("mouseleave", () => {
				tooltipAgent = null;
			})
			.call(d3.drag<SVGGElement, GraphNode>()
				.on("start", (event, d) => { d.fx = d.x; d.fy = d.y; })
				.on("drag", (event, d) => { d.fx = event.x; d.fy = event.y; })
				.on("end", () => { sim?.alphaTarget(0); }) as any,
			);

		const r = 20;

		nodeGroup.append("circle").attr("class", "glow-ring").attr("r", r + 6)
			.attr("fill", "none")
			.attr("stroke", (d: GraphNode) => roleColor(d.role))
			.attr("stroke-width", 2)
			.attr("opacity", 0.08)
			.attr("filter", "url(#g-glow)");

		nodeGroup.append("circle").attr("class", "node-body").attr("r", r)
			.style("fill", (d: GraphNode) => roleColor(d.role))
			.attr("fill-opacity", 0.85)
			.attr("stroke", (d: GraphNode) => roleColor(d.role))
			.attr("stroke-width", (d: GraphNode) => (d.id === selected ? 2.5 : 1))
			.attr("stroke-opacity", (d: GraphNode) => (d.id === selected ? 1 : 0.35));

		nodeGroup.append("circle").attr("class", "pulse-ring").attr("r", r + 5)
			.attr("fill", "none")
			.attr("stroke", (d: GraphNode) => roleColor(d.role))
			.attr("stroke-width", 2)
			.attr("opacity", 0);

		nodeGroup.append("text").attr("class", "node-id").attr("text-anchor", "middle").attr("dy", "0.35em")
			.attr("fill", "white").attr("font-size", "11").attr("font-family", "monospace").attr("font-weight", "700")
			.text((d: GraphNode) => `#${d.id}`);

		nodeGroup.append("text").attr("class", "node-role").attr("text-anchor", "middle")
			.attr("dy", (d: GraphNode) => (d.id === selected ? r + 14 : r + 11))
			.attr("fill", "currentColor").attr("font-size", "9").attr("opacity", 0.5)
			.text((d: GraphNode) => d.role);

		sim = d3.forceSimulation(nodes)
			.force("charge", d3.forceManyBody().strength(-180))
			.force("collision", d3.forceCollide().radius(42))
			.force("center", d3.forceCenter(width / 2, height / 2))
			.alphaDecay(0.08)
			.velocityDecay(0.65)
			.on("tick", () => {
				edgeLines
					.attr("x1", (e: GraphEdge) => sim!.nodes().find(n => n.id === e.source)!.x!)
					.attr("y1", (e: GraphEdge) => sim!.nodes().find(n => n.id === e.source)!.y!)
					.attr("x2", (e: GraphEdge) => sim!.nodes().find(n => n.id === e.target)!.x!)
					.attr("y2", (e: GraphEdge) => sim!.nodes().find(n => n.id === e.target)!.y!);
				nodeGroup.attr("transform", (d: GraphNode) => `translate(${d.x},${d.y})`);
			});

		sim.alpha(1).restart();
		applyStatusPulses();
	}

	function applyStatusPulses() {
		if (!svgEl) return;
		const svg = d3.select(svgEl);
		for (const [id, status] of statuses) {
			const pulse = svg.selectAll<SVGCircleElement, GraphNode>(".agent-node")
				.filter((d: GraphNode) => d.id === id)
				.select(".pulse-ring");
			if (status === "thinking" || status === "running-tool") {
				pulse.attr("opacity", 1);
				if (pulse.select("animate").empty()) {
					pulse.append("animate").attr("attributeName", "r")
						.attr("values", `${20 + 5};${20 + 12};${20 + 5}`)
						.attr("dur", "1.8s").attr("repeatCount", "indefinite");
					pulse.append("animate").attr("attributeName", "opacity")
						.attr("values", "0.5;0;0.5")
						.attr("dur", "1.8s").attr("repeatCount", "indefinite");
				}
				svg.selectAll<SVGCircleElement, GraphNode>(".agent-node")
					.filter((d: GraphNode) => d.id === id)
					.select(".node-body")
					.attr("fill-opacity", 0.95);
			} else {
				pulse.attr("opacity", 0);
				pulse.selectAll("animate").remove();
				svg.selectAll<SVGCircleElement, GraphNode>(".agent-node")
					.filter((d: GraphNode) => d.id === id)
					.select(".node-body")
					.attr("fill-opacity", 0.85);
			}
		}
	}

	function syncNodes() {
		if (!sim || !svgEl) return;
		const svgNodes = sim.nodes() as GraphNode[];
		const live = getNodes();

		const ids = new Set(live.map(n => n.id));

		// Remove dead nodes from cache; preserve positions for survivors
		for (const n of svgNodes) {
			if (!ids.has(n.id)) {
				nodeCache.delete(n.id);
			} else {
				const ln = live.find(l => l.id === n.id);
				if (ln) { ln.x = n.x; ln.y = n.y; }
			}
		}

		// Position brand-new nodes
		for (const n of live) {
			if (!svgNodes.find(s => s.id === n.id)) {
				n.x = containerEl!.clientWidth / 2 + (Math.random() - 0.5) * 60;
				n.y = containerEl!.clientHeight / 2 + (Math.random() - 0.5) * 60;
			}
		}

		sim.nodes(live);

		const edges = getEdges(live);
		edgeLines = edgeLines
			.data(edges, (e: GraphEdge) => `${e.source}-${e.target}`)
			.join(
				enter => enter.append("line")
					.attr("stroke", "currentColor")
					.attr("stroke-opacity", 0.08)
					.attr("stroke-width", 1),
				update => update,
				exit => exit.remove(),
			);

		nodeGroup = nodeGroup
			.data(live, (d: GraphNode) => String(d.id))
			.join(
				enter => {
					const gEnter = enter.append("g")
						.attr("class", "agent-node cursor-pointer")
						.attr("data-id", (d: GraphNode) => d.id)
						.on("click", (_event: any, d: GraphNode) => { onSelect(d.id); })
						.on("mouseenter", (_event: any, d: GraphNode) => {
							tooltipAgent = agents.find(a => a.id === d.id) ?? null;
						})
						.on("mouseleave", () => { tooltipAgent = null; })
						.call(d3.drag<SVGGElement, GraphNode>()
							.on("start", (event, d) => { d.fx = d.x; d.fy = d.y; })
							.on("drag", (event, d) => { d.fx = event.x; d.fy = event.y; })
							.on("end", () => { sim?.alphaTarget(0); }) as any);

					gEnter.append("circle").attr("class", "glow-ring").attr("r", 26)
						.attr("fill", "none")
						.attr("stroke", (d: GraphNode) => roleColor(d.role))
						.attr("stroke-width", 2)
						.attr("opacity", 0.08)
						.attr("filter", "url(#g-glow)");

					gEnter.append("circle").attr("class", "node-body").attr("r", 20)
						.style("fill", (d: GraphNode) => roleColor(d.role))
						.attr("fill-opacity", 0.85)
						.attr("stroke", (d: GraphNode) => roleColor(d.role))
						.attr("stroke-width", (d: GraphNode) => (d.id === selected ? 2.5 : 1))
						.attr("stroke-opacity", (d: GraphNode) => (d.id === selected ? 1 : 0.35));

					gEnter.append("circle").attr("class", "pulse-ring").attr("r", 25)
						.attr("fill", "none")
						.attr("stroke", (d: GraphNode) => roleColor(d.role))
						.attr("stroke-width", 2)
						.attr("opacity", 0);

					gEnter.append("text").attr("class", "node-id")
						.attr("text-anchor", "middle").attr("dy", "0.35em")
						.attr("fill", "white").attr("font-size", "11")
						.attr("font-family", "monospace").attr("font-weight", "700")
						.text((d: GraphNode) => `#${d.id}`);

					gEnter.append("text").attr("class", "node-role")
						.attr("text-anchor", "middle")
						.attr("dy", (d: GraphNode) => (d.id === selected ? 34 : 31))
						.attr("fill", "currentColor").attr("font-size", "9").attr("opacity", 0.5)
						.text((d: GraphNode) => d.role);

					return gEnter;
				},
				update => {
					update.select(".node-body")
						.style("fill", (d: GraphNode) => roleColor(d.role))
						.attr("stroke", (d: GraphNode) => roleColor(d.role))
						.attr("stroke-width", (d: GraphNode) => (d.id === selected ? 2.5 : 1))
						.attr("stroke-opacity", (d: GraphNode) => (d.id === selected ? 1 : 0.35));
					update.select(".node-role")
						.attr("dy", (d: GraphNode) => (d.id === selected ? 34 : 31));
					return update;
				},
				exit => exit.remove(),
			);

		sim.alpha(0.3).restart();
	}

	$effect(() => {
		if (agents.length === 0) { tooltipAgent = null; }
	});

	$effect(() => {
		if (svgEl && agents.length > 0) {
			if (!sim) {
				init();
			} else {
				syncNodes();
			}
			applyStatusPulses();
			applySelection();
		}
	});

	$effect(() => { selected; if (svgEl && sim) applySelection(); });
	$effect(() => { statuses; if (svgEl && sim) applyStatusPulses(); });

	function applySelection() {
		if (!svgEl) return;
		const svg = d3.select(svgEl);
		svg.selectAll<SVGCircleElement, GraphNode>(".node-body")
			.attr("stroke-width", (d: GraphNode) => (d.id === selected ? 2.5 : 1))
			.attr("stroke-opacity", (d: GraphNode) => (d.id === selected ? 1 : 0.35));
		svg.selectAll<SVGTextElement, GraphNode>(".node-role")
			.attr("dy", (d: GraphNode) => (d.id === selected ? 34 : 31));
	}

	onMount(() => {
		let rafId: number | null = null;
		ro = new ResizeObserver((entries) => {
			if (rafId != null) cancelAnimationFrame(rafId);
			rafId = requestAnimationFrame(() => {
				rafId = null;
				if (!containerEl || !sim) return;
				const entry = entries[0];
				const w = entry.contentRect.width;
				const h = entry.contentRect.height;
				if (w === 0 || h === 0) return;
				const center = sim.force("center") as d3.ForceCenter<GraphNode> | undefined;
				if (center) center.x(w / 2).y(h / 2);
			});
		});
		ro.observe(containerEl);
		return () => { ro.disconnect(); if (rafId != null) cancelAnimationFrame(rafId); if (sim) sim.stop(); };
	});
</script>

<div bind:this={containerEl} class="size-full relative bg-background overflow-hidden">
	{#if agents.length === 0}
		<div class="absolute inset-0 flex items-center justify-center pointer-events-none">
			<p class="text-xs text-muted-foreground/50">No agents yet</p>
		</div>
	{/if}

	<svg bind:this={svgEl} class="size-full" role="img" aria-label="Agent graph" />

	{#if tooltipAgent}
		{@const st = statuses.get(tooltipAgent.id) ?? "idle"}
		{@const sc = st === "thinking" || st === "running-tool" ? "#f59e0b" : st === "responding" ? "#22c55e" : st === "error" ? "#ef4444" : "#6b7280"}
		<div class="absolute top-2 left-2 flex items-center gap-2 px-2.5 py-1.5 rounded-md bg-background/90 border border-border text-xs shadow-sm backdrop-blur-sm">
			<div class="size-2 rounded-full" style="background: {sc}"></div>
			<span class="font-medium">#{tooltipAgent.id} {tooltipAgent.role}</span>
			<span class="text-muted-foreground/60 text-[10px] capitalize">{st.replace("-", " ")}</span>
		</div>
	{/if}

	{#if uniqueRoles.length > 1}
		<div class="absolute bottom-2 left-2 right-2 flex flex-wrap gap-x-3 gap-y-1 justify-center">
			{#each uniqueRoles as role}
				<div class="flex items-center gap-1.5">
					<div class="size-2 rounded-full" style="background: {roleColor(role)}"></div>
					<span class="text-[10px] text-muted-foreground/60">{role}</span>
				</div>
			{/each}
		</div>
	{/if}
</div>
