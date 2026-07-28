<script lang="ts">
	import { Button } from "$lib/components/ui/button";
	import { Badge } from "$lib/components/ui/badge";
	import { Tooltip, TooltipContent, TooltipTrigger } from "$lib/components/ui/tooltip";
	import {
		Plus,
		X,
		ChevronRight,
		ChevronDown,
		Plug,
		Terminal,
		Globe,
		Shield,
		Loader2,
	} from "@lucide/svelte";
	import { cn } from "$lib/utils.js";
	import type { McpConnectionInfo, McpServerConfig } from "$lib/types";
	import McpAddForm from "./mcp-add-form.svelte";

	let {
		configs,
		connections,
		expanded,
		onToggle,
		onAdd,
		onRemove,
	}: {
		configs: McpServerConfig[];
		connections: McpConnectionInfo[];
		expanded: boolean;
		onToggle: () => void;
		onAdd: (config: McpServerConfig) => void;
		onRemove: (name: string) => void;
	} = $props();

	let showAddForm = $state(false);
	let adding = $state(false);

	function transportLabel(t: McpServerConfig["transport"]): string {
		if (t.type === "stdio") return `${t.command} ${t.args.join(" ")}`;
		return t.url;
	}

	function isConnected(name: string): boolean {
		return connections.some((c) => c.name === name);
	}

	function getConnection(name: string): McpConnectionInfo | undefined {
		return connections.find((c) => c.name === name);
	}

	function toggleAddForm() {
		showAddForm = !showAddForm;
	}

	async function handleAdd(config: McpServerConfig) {
		adding = true;
		try {
			await onAdd(config);
			showAddForm = false;
		} catch {
			// error handled by parent
		} finally {
			adding = false;
		}
	}

	function handleCancelAdd() {
		showAddForm = false;
	}
</script>

<div class="shrink-0 border-t border-border/40">
	<button
		class="w-full flex items-center justify-between px-3 py-2 text-xs font-semibold text-muted-foreground uppercase tracking-wider hover:bg-muted/30 transition-colors"
		onclick={onToggle}
	>
		<div class="flex items-center gap-2">
			<Plug class="size-3.5" />
			<span>MCP</span>
			<Badge variant="secondary" class="text-[10px] px-1.5 py-0 h-4 min-w-0">
				{connections.length}
			</Badge>
		</div>
		<div class="flex items-center gap-1">
			<Button
				variant="ghost"
				size="icon-xs"
				onclick={(e) => {
					e.stopPropagation();
					toggleAddForm();
				}}
				title="Add MCP server"
				aria-label="Add MCP server"
			>
				<Plus class="size-3" />
			</Button>
			{#if expanded}
				<ChevronDown class="size-3 transition-transform" />
			{:else}
				<ChevronRight class="size-3 transition-transform" />
			{/if}
		</div>
	</button>

	{#if expanded}
		<div class="px-3 pb-2 flex flex-col gap-1 max-h-48 overflow-y-auto no-scrollbar">
			{#if configs.length === 0 && connections.length === 0}
				<div class="flex flex-col items-center gap-2 py-6 text-center px-2">
					<p class="text-xs text-muted-foreground">No MCP servers</p>
				</div>
			{:else}
				{#each configs as config (config.name)}
					{@const connected = isConnected(config.name)}
					{@const conn = getConnection(config.name)}
					<div
						class="flex items-start gap-2 px-2 py-1.5 rounded-md text-xs group relative hover:bg-muted/30 transition-colors"
					>
						<div class="relative shrink-0 mt-1">
							<div
								class={cn(
									"w-1.5 h-1.5 rounded-full mt-0.5",
									connected ? "bg-emerald-500" : "bg-muted-foreground/30",
								)}
							></div>
						</div>
						<div class="flex-1 min-w-0">
							<div class="flex items-center gap-1.5">
								<span class={cn("font-medium truncate", !connected && "text-muted-foreground/60")}>
									{config.name}
								</span>
							</div>
							{#if conn}
								<div class="text-[11px] text-muted-foreground/70 mt-0.5 leading-tight">
									{conn.tool_names.length} tool{conn.tool_names.length !== 1 ? "s" : ""}
								</div>
								{#if conn.tool_names.length > 0}
									<div class="flex flex-wrap gap-1 mt-1">
										{#each conn.tool_names.slice(0, 6) as tool}
											<span class="text-[10px] px-1 py-0.5 rounded bg-muted/50 text-muted-foreground/70 truncate max-w-24">
												{tool}
											</span>
										{/each}
										{#if conn.tool_names.length > 6}
											<span class="text-[10px] text-muted-foreground/50">+{conn.tool_names.length - 6}</span>
										{/if}
									</div>
								{/if}
							{:else}
								<div class="text-[11px] text-muted-foreground/40 mt-0.5 leading-tight flex items-center gap-1">
									{#if config.transport.type === "stdio"}
										<Terminal class="size-2.5" />
									{:else}
										<Globe class="size-2.5" />
									{/if}
									<span class="truncate">{transportLabel(config.transport)}</span>
								</div>
							{/if}
						</div>
						<div class="absolute right-0.5 inset-y-0 flex items-center opacity-0 group-hover:opacity-100 transition-opacity">
							{#if config.dangerous_tools && config.dangerous_tools.length > 0}
								<Tooltip>
									<TooltipTrigger>
										<Shield class="size-2.5 text-amber-500 mr-0.5" />
									</TooltipTrigger>
									<TooltipContent side="right" class="max-w-36">
										<p class="text-[11px]">{config.dangerous_tools.join(", ")}</p>
									</TooltipContent>
								</Tooltip>
							{/if}
							<Button
								variant="ghost"
								size="icon-xs"
								class="text-muted-foreground hover:text-destructive"
								onclick={(e) => { e.stopPropagation(); onRemove(config.name); }}
								title="Remove MCP server"
							>
								<X class="size-3" />
							</Button>
						</div>
					</div>
				{/each}
			{/if}

			{#if showAddForm}
				<McpAddForm
					{adding}
					onAdd={handleAdd}
					onCancel={handleCancelAdd}
				/>
			{/if}
		</div>
	{/if}
</div>
