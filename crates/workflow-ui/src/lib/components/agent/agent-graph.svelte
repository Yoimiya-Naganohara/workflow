<script lang="ts">
	import { onMount } from "svelte";
	import * as d3 from "d3";
	import { Button } from "$lib/components/ui/button";
	import { formatRole } from "$lib/utils.js";
	import type { AgentInfo, AgentId, AgentStatus } from "$lib/types";

	interface GraphNode extends d3.SimulationNodeDatum {
		id: number;
		role: string;
		task: string | null;
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

	const nodeCache = new Map<number, GraphNode>();

	function getNodes(): GraphNode[] {
		return agents.map(a => {
			let n = nodeCache.get(a.id);
			if (!n) {
				n = { id: a.id, role: a.role, task: a.current_task, x: NaN, y: NaN };
				nodeCache.set(a.id, n);
			}
			n.role = a.role;
			n.task = a.current_task;
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

	function roleIndex(role: string): number {
		return uniqueRoles.indexOf(role);
	}

	function buildNodeEnter(enter: d3.Selection<d3.EnterElement, GraphNode, SVGGElement, unknown>) {
		const gEnter = enter.append("g")
			.attr("class", "agent-node")
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
			.attr("stroke-width", 2).attr("opacity", 0.08)
			.attr("filter", "url(#g-glow)");

		gEnter.append("circle").attr("class", "node-body").attr("r", 20)
			.style("fill", (d: GraphNode) => roleColor(d.role))
			.attr("fill-opacity", 0.85)
			.attr("stroke", (d: GraphNode) => roleColor(d.role))
			.attr("stroke-width", 1.5)
			.attr("stroke-opacity", 0.35);

		gEnter.append("circle").attr("class", "status-ring").attr("r", 24)
			.attr("fill", "none")
			.attr("stroke-width", 2.5)
			.attr("stroke-opacity", 0)
			.attr("stroke-linecap", "round")
			.style("stroke-dasharray", "0 151");

		gEnter.append("circle").attr("class", "pulse-ring").attr("r", 25)
			.attr("fill", "none")
			.attr("stroke", (d: GraphNode) => roleColor(d.role))
			.attr("stroke-width", 2).attr("opacity", 0);

		gEnter.append("text").attr("class", "node-id")
			.attr("text-anchor", "middle").attr("dy", "0.35em")
			.attr("fill", "white").attr("font-size", "11")
			.attr("font-family", "monospace").attr("font-weight", "700")
			.text((d: GraphNode) => `#${d.id}`);

		gEnter.append("text").attr("class", "node-role")
			.attr("text-anchor", "middle")
			.attr("dy", 31)
			.attr("fill", "currentColor").attr("font-size", "9").attr("opacity", 0.5)
			.text((d: GraphNode) => formatRole(d.role));

		gEnter.append("text").attr("class", "node-task")
			.attr("text-anchor", "middle")
			.attr("dy", 42)
			.attr("fill", "currentColor").attr("font-size", "7").attr("opacity", 0.35)
			.style("pointer-events", "none")
			.each(function (d: GraphNode) {
				const el = d3.select(this);
				if (d.task) {
					const text = d.task.length > 28 ? d.task.slice(0, 28) + "…" : d.task;
					el.text(text);
				}
			});

		return gEnter;
	}

	function buildNodeUpdate(update: d3.Selection<SVGGElement, GraphNode, SVGGElement, unknown>) {
		update.select(".node-body")
			.style("fill", (d: GraphNode) => roleColor(d.role))
			.attr("stroke", (d: GraphNode) => roleColor(d.role));
		update.select(".node-task").each(function (d: GraphNode) {
			const el = d3.select(this);
			if (d.task) {
				const text = d.task.length > 28 ? d.task.slice(0, 28) + "…" : d.task;
				el.text(text).attr("opacity", 0.35);
			} else {
				el.text("").attr("opacity", 0);
			}
		});
		return update;
	}

	let g: d3.Selection<SVGGElement, unknown, null, undefined>;
	let nodeGroup: d3.Selection<SVGGElement, GraphNode, SVGGElement, unknown>;
	let edgeLines: d3.Selection<SVGLineElement, GraphEdge, SVGGElement, unknown>;

	function init() {
		if (!svgEl || !containerEl || agents.length === 0) return;

		if (sim) { sim.stop(); sim = null; }
		nodeCache.clear();

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

		defs.append("marker")
			.attr("id", "arrowhead")
			.attr("viewBox", "0 0 10 10")
			.attr("refX", "28")
			.attr("refY", "5")
			.attr("markerWidth", "6")
			.attr("markerHeight", "6")
			.attr("orient", "auto")
			.append("path")
			.attr("d", "M 0 0 L 10 5 L 0 10 z")
			.attr("fill", "currentColor")
			.attr("opacity", 0.15);

		g = svg.append("g").attr("class", "graph-root");

		const zoom = d3.zoom<SVGSVGElement, unknown>()
			.extent([[0, 0], [width, height]])
			.scaleExtent([0.1, 4])
			.on("zoom", (event) => {
				g.attr("transform", event.transform.toString());
				zoomLevel = event.transform.k;
			});
		svg.call(zoom);

		const nodes = getNodes();
		const edges = getEdges(nodes);
		const rc = uniqueRoles.length;

		edgeLines = g.selectAll<SVGLineElement, GraphEdge>("line")
			.data(edges)
			.join("line")
			.attr("stroke", "currentColor")
			.attr("stroke-opacity", 0.06)
			.attr("stroke-width", 1)
			.attr("marker-end", "url(#arrowhead)");

		nodeGroup = g.selectAll<SVGGElement, GraphNode>("g")
			.data(nodes, (d: GraphNode) => String(d.id))
			.join("g")
			.call(gEnter => { buildNodeEnter(gEnter as any); });

		sim = d3.forceSimulation(nodes)
			.force("charge", d3.forceManyBody().strength(-180))
			.force("collision", d3.forceCollide().radius(42))
			.force("x", d3.forceX((d: GraphNode) => {
				const idx = roleIndex(d.role);
				return rc > 1 ? (width / (rc + 1)) * (idx + 1) : width / 2;
			}).strength(rc > 1 ? 0.15 : 0))
			.force("y", d3.forceY(height / 2).strength(0.05))
			.force("center", d3.forceCenter(width / 2, height / 2))
			.alphaDecay(0.08)
			.velocityDecay(0.65)
			.on("tick", () => {
				const simNodes = sim!.nodes();
				edgeLines
					.attr("x1", (e: GraphEdge) => {
						const n = simNodes.find(n => n.id === e.source);
						return n?.x ?? 0;
					})
					.attr("y1", (e: GraphEdge) => {
						const n = simNodes.find(n => n.id === e.source);
						return n?.y ?? 0;
					})
					.attr("x2", (e: GraphEdge) => {
						const n = simNodes.find(n => n.id === e.target);
						return n?.x ?? 0;
					})
					.attr("y2", (e: GraphEdge) => {
						const n = simNodes.find(n => n.id === e.target);
						return n?.y ?? 0;
					});
				nodeGroup.attr("transform", (d: GraphNode) => `translate(${d.x},${d.y})`);
			});

		sim.alpha(1).restart();
		applyStatusRings();
		applyStatusPulses();
		applySelection();
	}

	function applyStatusRings() {
		if (!svgEl) return;
		const svg = d3.select(svgEl);
		for (const [id, st] of statuses) {
			const node = svg.select(`[data-id="${id}"]`);
			if (node.empty()) continue;
			const ring = node.select(".status-ring");
			// Remove old animations first
			ring.selectAll("animate").remove();
			if (st !== "idle") {
				const color = statusColor(st);
				const circumference = 2 * Math.PI * 24;
				ring.attr("stroke", color)
					.attr("stroke-opacity", st === "error" ? 0.7 : 0.4)
					.style("stroke-dasharray", st === "error" ? `${circumference} ${circumference}` : `${circumference * 0.35} ${circumference}`);
				ring.append("animate").attr("attributeName", "stroke-dashoffset")
					.attr("from", "0").attr("to", st === "error" ? "0" : `-${circumference}`)
					.attr("dur", "2s").attr("repeatCount", "indefinite");
			} else {
				ring.attr("stroke-opacity", 0)
					.style("stroke-dasharray", "0 151");
			}
		}
	}

	function applyStatusPulses() {
		if (!svgEl) return;
		const svg = d3.select(svgEl);
		for (const [id, status] of statuses) {
			const node = svg.select(`[data-id="${id}"]`);
			if (node.empty()) continue;
			const pulse = node.select(".pulse-ring");
			const body = node.select(".node-body");
			// Remove old animations first
			pulse.selectAll("animate").remove();
			if (status === "thinking" || status === "running-tool") {
				pulse.attr("opacity", 1);
				pulse.append("animate").attr("attributeName", "r")
					.attr("values", "25;32;25")
					.attr("dur", "1.8s").attr("repeatCount", "indefinite");
				pulse.append("animate").attr("attributeName", "opacity")
					.attr("values", "0.5;0;0.5")
					.attr("dur", "1.8s").attr("repeatCount", "indefinite");
				body.attr("fill-opacity", 0.95);
			} else {
				pulse.attr("opacity", 0);
				body.attr("fill-opacity", 0.85);
			}
		}
	}

	function syncNodes() {
		if (!sim || !svgEl) return;
		const svgNodes = sim.nodes() as GraphNode[];
		const live = getNodes();

		const liveById = new Map(live.map(n => [n.id, n]));
		const oldById = new Map(svgNodes.map(n => [n.id, n]));

		for (const n of svgNodes) {
			if (!liveById.has(n.id)) {
				nodeCache.delete(n.id);
			} else {
				const ln = liveById.get(n.id)!;
				ln.x = n.x; ln.y = n.y;
			}
		}

		for (const n of live) {
			if (!oldById.has(n.id)) {
				n.x = containerEl!.clientWidth / 2 + (Math.random() - 0.5) * 60;
				n.y = containerEl!.clientHeight / 2 + (Math.random() - 0.5) * 60;
			}
		}

		sim.nodes(live);
		sim.force("x", d3.forceX((d: GraphNode) => {
			const idx = roleIndex(d.role);
			const w = containerEl!.clientWidth;
			const rc = uniqueRoles.length;
			return rc > 1 ? (w / (rc + 1)) * (idx + 1) : w / 2;
		}).strength(uniqueRoles.length > 1 ? 0.15 : 0));

		const edges = getEdges(live);
		edgeLines = edgeLines
			.data(edges, (e: GraphEdge) => `${e.source}-${e.target}`)
			.join(
				enter => enter.append("line")
					.attr("stroke", "currentColor")
					.attr("stroke-opacity", 0.06)
					.attr("stroke-width", 1)
					.attr("marker-end", "url(#arrowhead)"),
				update => update,
				exit => exit.remove(),
			);

		nodeGroup = nodeGroup
			.data(live, (d: GraphNode) => String(d.id))
			.join(
				enter => buildNodeEnter(enter),
				update => buildNodeUpdate(update),
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
			applyStatusRings();
			applySelection();
		} else if (svgEl && agents.length === 0 && sim) {
			sim.stop();
			sim = null;
			d3.select(svgEl).selectAll("*").remove();
			nodeCache.clear();
		}
	});

	$effect(() => { selected; if (svgEl && sim) applySelection(); });
	$effect(() => { statuses; if (svgEl && sim) { applyStatusPulses(); applyStatusRings(); } });

	let zoomLevel = $state(1);

	function fitToView() {
		if (!svgEl || !sim) return;
		const nodes = sim.nodes() as GraphNode[];
		if (nodes.length === 0) return;
		const width = containerEl!.clientWidth;
		const height = containerEl!.clientHeight;
		if (width === 0 || height === 0) return;
		const xs = nodes.map(n => n.x ?? 0);
		const ys = nodes.map(n => n.y ?? 0);
		const minX = Math.min(...xs), maxX = Math.max(...xs);
		const minY = Math.min(...ys), maxY = Math.max(...ys);
		const padding = 60;
		const bw = maxX - minX + padding * 2;
		const bh = maxY - minY + padding * 2;
		if (bw === 0 || bh === 0) return;
		const scale = Math.min(width / bw, height / bh, 2);
		const tx = width / 2 - (minX + maxX) / 2 * scale;
		const ty = height / 2 - (minY + maxY) / 2 * scale;
		const svg = d3.select(svgEl);
		svg.select(".graph-root")
			.transition()
			.duration(400)
			.attr("transform", `translate(${tx},${ty}) scale(${scale})`);
		zoomLevel = scale;
	}

	function applySelection() {
		if (!svgEl) return;
		d3.select(svgEl).selectAll<SVGGElement, GraphNode>(".agent-node")
			.select(".node-body")
			.attr("stroke-width", (d: GraphNode) => (d.id === selected ? 2.5 : 1.5))
			.attr("stroke-opacity", (d: GraphNode) => (d.id === selected ? 1 : 0.35));
		d3.select(svgEl).selectAll<SVGGElement, GraphNode>(".agent-node")
			.select(".node-role")
			.attr("dy", (d: GraphNode) => (d.id === selected ? 30 : 31));
		d3.select(svgEl).selectAll<SVGGElement, GraphNode>(".agent-node")
			.select(".glow-ring")
			.attr("opacity", (d: GraphNode) => (d.id === selected ? 0.2 : 0.08));
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
				if (document.hidden) return; // Skip when tab is hidden
				const center = sim.force("center") as d3.ForceCenter<GraphNode> | undefined;
				if (center) center.x(w / 2).y(h / 2);
				const fx = sim.force("x") as d3.ForceX<GraphNode> | undefined;
				if (fx && uniqueRoles.length > 1) {
					fx.x((d: GraphNode) => {
						const idx = roleIndex(d.role);
						return (w / (uniqueRoles.length + 1)) * (idx + 1);
					});
				}
				sim.alpha(0.2).restart();
			});
		});
		ro.observe(containerEl);

		// Stop simulation when tab is hidden
		const onVisibilityChange = () => {
			if (!sim) return;
			if (document.hidden) {
				sim.stop();
			} else {
				sim.alpha(0.1).restart();
			}
		};
		document.addEventListener("visibilitychange", onVisibilityChange);

		return () => {
			ro.disconnect();
			if (rafId != null) cancelAnimationFrame(rafId);
			if (sim) sim.stop();
			document.removeEventListener("visibilitychange", onVisibilityChange);
		};
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
		{@const sc = statusColor(st)}
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
					<span class="text-[10px] text-muted-foreground/60">{formatRole(role)}</span>
				</div>
			{/each}
		</div>
	{/if}

	<div class="absolute top-2 right-2 flex items-center gap-1">
		{#if agents.length > 1 && zoomLevel !== 1}
			<Button variant="ghost" size="icon-xs" onclick={fitToView} title="Fit to view">
				<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-3.5"><path d="M15 3h6v6"/><path d="M9 21H3v-6"/><path d="M21 3l-7 7"/><path d="M3 21l7-7"/></svg>
			</Button>
		{/if}
	</div>
</div>