<script lang="ts">
	import { onMount } from "svelte";
	import * as d3 from "d3";
	import type { AgentInfo, AgentId, AgentStatus } from "$lib/types";

	interface GraphNode extends d3.SimulationNodeDatum {
		id: number;
		role: string;
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
	let hoveredId = $state<AgentId | null>(null);
	let tooltipAgent = $state<AgentInfo | null>(null);

	const palette: Record<string, { fill: string; glow: string }> = {
		planner:   { fill: "#6366f1", glow: "rgba(99,102,241,0.25)" },
		executor:  { fill: "#22c55e", glow: "rgba(34,197,94,0.25)" },
		reviewer:  { fill: "#f59e0b", glow: "rgba(245,158,11,0.25)" },
		reporter:  { fill: "#06b6d4", glow: "rgba(6,182,212,0.25)" },
		default:   { fill: "#a78bfa", glow: "rgba(167,139,250,0.25)" },
	};

	const uniqueRoles = $derived([...new Set(agents.map(a => a.role))]);

	function style(role: string) {
		return palette[role] ?? palette.default;
	}

	function rebuild() {
		if (!svgEl || !containerEl) return;

		const svg = d3.select(svgEl);

		if (agents.length === 0) return;

		if (sim) { sim.stop(); sim = null; }

		svg.selectAll("*").remove();

		const width = containerEl.clientWidth;
		const height = containerEl.clientHeight;

		const defs = svg.append("defs");

		for (const [role, c] of Object.entries(palette)) {
			const g = defs.append("radialGradient").attr("id", `g-${role}`).attr("cx", "35%").attr("cy", "35%").attr("r", "65%");
			g.append("stop").attr("offset", "0%").attr("stop-color", "#fff").attr("stop-opacity", "0.35");
			g.append("stop").attr("offset", "100%").attr("stop-color", c.fill).attr("stop-opacity", "1");
		}

		const flt = defs.append("filter").attr("id", "gl").attr("x", "-50%").attr("y", "-50%").attr("width", "200%").attr("height", "200%");
		flt.append("feGaussianBlur").attr("stdDeviation", "3").attr("result", "b");
		const m = flt.append("feMerge");
		m.append("feMergeNode").attr("in", "b");
		m.append("feMergeNode").attr("in", "SourceGraphic");

		const g = svg.append("g").attr("class", "graph-root");

		const zoom = d3.zoom<SVGSVGElement, unknown>()
			.extent([[0, 0], [width, height]])
			.scaleExtent([0.1, 4])
			.on("zoom", (event) => g.attr("transform", event.transform.toString()));
		svg.call(zoom);

		const nodes: GraphNode[] = agents.map((a) => ({ id: a.id, role: a.role }));
		const pairs = d3.pairs(nodes);

		const edge = g.selectAll<SVGLineElement, [GraphNode, GraphNode]>("line")
			.data(pairs)
			.join("line")
			.attr("stroke", "currentColor")
			.attr("stroke-opacity", 0.06)
			.attr("stroke-width", 1);

		const r = 22;

		const node = g.selectAll<SVGGElement, GraphNode>("g")
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
			.on("mouseenter", (_event: any, d: GraphNode) => {
				hoveredId = d.id;
				tooltipAgent = agents.find(a => a.id === d.id) ?? null;
				d3.select(svgEl).selectAll<SVGGElement, GraphNode>(".agent-node")
					.attr("opacity", (n: GraphNode) => (n.id === d.id ? 1 : 0.5));
			})
			.on("mouseleave", () => {
				hoveredId = null;
				tooltipAgent = null;
				d3.select(svgEl).selectAll(".agent-node").attr("opacity", 1);
			})
			.call(d3.drag<SVGGElement, GraphNode>()
				.on("start", (event, d) => { d.fx = d.x; d.fy = d.y; })
				.on("drag", (event, d) => { if (sim) { sim.alpha(0.01); sim.restart(); } d.fx = event.x; d.fy = event.y; })
				.on("end", () => { sim?.alphaTarget(0); }) as any,
			);

		node.append("circle").attr("class", "glow-ring").attr("r", r + 4)
			.attr("fill", (d: GraphNode) => style(d.role).glow)
			.attr("filter", "url(#gl)").attr("opacity", 0.5);

		node.append("circle").attr("class", "node-body").attr("r", r)
			.attr("fill", (d: GraphNode) => `url(#g-${d.role})`)
			.attr("stroke", (d: GraphNode) => style(d.role).fill)
			.attr("stroke-width", (d: GraphNode) => (d.id === selected ? 3 : 1.5))
			.attr("stroke-opacity", (d: GraphNode) => (d.id === selected ? 1 : 0.5));

		node.append("circle").attr("class", "pulse-ring").attr("r", r + 6)
			.attr("fill", "none").attr("stroke", (d: GraphNode) => style(d.role).fill)
			.attr("stroke-width", 2).attr("opacity", 0);

		node.append("text").attr("class", "node-id").attr("text-anchor", "middle").attr("dy", "0.35em")
			.attr("fill", "white").attr("font-size", "12").attr("font-family", "monospace").attr("font-weight", "700")
			.text((d: GraphNode) => `#${d.id}`);

		node.append("text").attr("class", "node-role").attr("text-anchor", "middle")
			.attr("dy", (d: GraphNode) => (d.id === selected ? r + 16 : r + 12))
			.attr("fill", "currentColor").attr("font-size", "10").attr("opacity", 0.6)
			.text((d: GraphNode) => d.role);

		sim = d3.forceSimulation(nodes)
			.force("center", d3.forceCenter(width / 2, height / 2))
			.force("charge", d3.forceManyBody().strength(-300))
			.force("collision", d3.forceCollide().radius(50))
			.on("tick", () => {
				edge.attr("x1", (d: any) => d[0].x).attr("y1", (d: any) => d[0].y)
					.attr("x2", (d: any) => d[1].x).attr("y2", (d: any) => d[1].y);
				node.attr("transform", (d: GraphNode) => `translate(${d.x},${d.y})`);
			});

		sim.alpha(1).restart();
		updateStatusPulses(svg);
	}

	function updateSelection() {
		if (!svgEl) return;
		const svg = d3.select(svgEl);
		svg.selectAll<SVGCircleElement, GraphNode>(".node-body")
			.attr("stroke-width", (d: GraphNode) => (d.id === selected ? 3 : 1.5))
			.attr("stroke-opacity", (d: GraphNode) => (d.id === selected ? 1 : 0.5));
		svg.selectAll<SVGTextElement, GraphNode>(".node-role")
			.attr("dy", (d: GraphNode) => (d.id === selected ? 38 : 34));
	}

	function updateStatusPulses(svg?: d3.Selection<SVGSVGElement, unknown, null, undefined>) {
		if (!svg) svg = d3.select(svgEl);
		if (svg.empty()) return;
		for (const [id, status] of statuses) {
			const pulse = svg.selectAll<SVGCircleElement, GraphNode>(".agent-node")
				.filter((d: GraphNode) => d.id === id)
				.select(".pulse-ring");
			if (status === "thinking" || status === "running-tool") {
				pulse.attr("opacity", 1);
				if (pulse.select("animate").empty()) {
					pulse.append("animate").attr("attributeName", "r")
						.attr("values", `${22 + 6};${22 + 14};${22 + 6}`)
						.attr("dur", "1.8s").attr("repeatCount", "indefinite");
					pulse.append("animate").attr("attributeName", "opacity")
						.attr("values", "0.6;0;0.6")
						.attr("dur", "1.8s").attr("repeatCount", "indefinite");
				}
				svg.selectAll<SVGCircleElement, GraphNode>(".agent-node")
					.filter((d: GraphNode) => d.id === id)
					.select(".node-body")
					.attr("opacity", 0.8);
			} else {
				pulse.attr("opacity", 0);
				pulse.selectAll("animate").remove();
				svg.selectAll<SVGCircleElement, GraphNode>(".agent-node")
					.filter((d: GraphNode) => d.id === id)
					.select(".node-body")
					.attr("opacity", 1);
			}
		}
	}

	$effect(() => {
		if (agents.length === 0) { tooltipAgent = null; }
	});

	$effect(() => { selected; if (svgEl) updateSelection(); });
	$effect(() => { statuses; if (svgEl) updateStatusPulses(); });

	onMount(() => {
		let rafId: number | null = null;
		requestAnimationFrame(() => {
			if (svgEl && agents.length > 0) rebuild();
		});
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
					<div class="size-2 rounded-full" style="background: {style(role).fill}"></div>
					<span class="text-[10px] text-muted-foreground/60">{role}</span>
				</div>
			{/each}
		</div>
	{/if}
</div>
