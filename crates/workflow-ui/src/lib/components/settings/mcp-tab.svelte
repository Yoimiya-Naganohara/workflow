<script lang="ts">
	import { Button } from "$lib/components/ui/button";
	import { Card } from "$lib/components/ui/card";
	import { Badge } from "$lib/components/ui/badge";
	import { Tooltip, TooltipContent, TooltipTrigger } from "$lib/components/ui/tooltip";
	import {
		Plug,
		Plus,
		X,
		Terminal,
		Globe,
		Shield,
		Loader2,
	} from "@lucide/svelte";
	import { cn } from "$lib/utils";
	import type { McpConnectionInfo, McpServerConfig } from "$lib/types";
	import McpAddForm from "$lib/components/agent/mcp-add-form.svelte";

	let {
		configs,
		connections,
		onAdd,
		onRemove,
	}: {
		configs: McpServerConfig[];
		connections: McpConnectionInfo[];
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

<div class="max-w-xl mx-auto space-y-6">
	<div>
		<h2 class="text-base font-semibold">MCP Servers</h2>
		<p class="text-xs text-muted-foreground/60 mt-0.5">
			Manage Model Context Protocol servers for tool access.
		</p>
	</div>

	<Card class="p-4 space-y-4">
		<div class="flex items-center justify-between">
			<div class="flex items-center gap-2">
				<span class="text-xs font-medium text-muted-foreground">Configured Servers</span>
				<Badge variant="secondary" class="text-[10px] px-1.5 py-0 h-4 min-w-0">
					{connections.length}
				</Badge>
			</div>
			<Button variant="outline" size="xs" onclick={() => (showAddForm = !showAddForm)} disabled={adding}>
				<Plus class="size-3" />
				Add Server
			</Button>
		</div>

		{#if configs.length === 0 && connections.length === 0}
			<div class="flex flex-col items-center gap-3 py-12 text-center">
				<div class="size-10 rounded-full bg-muted/50 flex items-center justify-center">
					<Plug class="size-4 text-muted-foreground/40" />
				</div>
				<p class="text-xs text-muted-foreground/60">No MCP servers configured yet.</p>
				<Button variant="outline" size="xs" onclick={() => (showAddForm = true)}>
					<Plus class="size-3" /> Add your first server
				</Button>
			</div>
		{:else}
			<div class="space-y-1">
				{#each configs as config (config.name)}
					{@const connected = isConnected(config.name)}
					{@const conn = getConnection(config.name)}
					<div
						class="flex items-start gap-3 px-3 py-2 rounded-lg text-xs group relative hover:bg-muted/30 transition-colors"
					>
						<div class="relative shrink-0 mt-1">
							<div
								class={cn(
									"size-2 rounded-full",
									connected ? "bg-emerald-500" : "bg-muted-foreground/30",
								)}
							></div>
						</div>
						<div class="flex-1 min-w-0">
							<div class="flex items-center gap-2">
								<span class={cn("font-medium", !connected && "text-muted-foreground/60")}>
									{config.name}
								</span>
								{#if connected}
									<Badge variant="outline" class="text-[10px] px-1 py-px font-normal text-emerald-600 dark:text-emerald-400">
										Connected
									</Badge>
								{/if}
							</div>
							{#if conn}
								<div class="text-[11px] text-muted-foreground/70 mt-1">
									{conn.tool_names.length} tool{conn.tool_names.length !== 1 ? "s" : ""}
								</div>
								{#if conn.tool_names.length > 0}
									<div class="flex flex-wrap gap-1 mt-1.5">
										{#each conn.tool_names.slice(0, 8) as tool}
											<span class="text-[10px] px-1.5 py-0.5 rounded bg-muted/50 text-muted-foreground/70 truncate max-w-32">
												{tool}
											</span>
										{/each}
										{#if conn.tool_names.length > 8}
											<span class="text-[10px] text-muted-foreground/50">+{conn.tool_names.length - 8}</span>
										{/if}
									</div>
								{/if}
							{:else}
								<div class="text-[11px] text-muted-foreground/40 mt-1 flex items-center gap-1">
									{#if config.transport.type === "stdio"}
										<Terminal class="size-2.5" />
									{:else}
										<Globe class="size-2.5" />
									{/if}
									<span class="truncate">{transportLabel(config.transport)}</span>
								</div>
							{/if}
						</div>
						<div class="flex items-center gap-1 shrink-0">
							{#if config.dangerous_tools && config.dangerous_tools.length > 0}
								<Tooltip>
									<TooltipTrigger>
										<Shield class="size-3 text-amber-500" />
									</TooltipTrigger>
									<TooltipContent side="left" class="max-w-36">
										<p class="text-[11px]">{config.dangerous_tools.join(", ")}</p>
									</TooltipContent>
								</Tooltip>
							{/if}
							<Button
								variant="ghost"
								size="icon-xs"
								class="text-muted-foreground hover:text-destructive"
								onclick={() => onRemove(config.name)}
								title="Remove MCP server"
							>
								<X class="size-3" />
							</Button>
						</div>
					</div>
				{/each}
			</div>
		{/if}

		{#if showAddForm}
			<McpAddForm
				{adding}
				onAdd={handleAdd}
				onCancel={handleCancelAdd}
			/>
		{/if}
	</Card>
</div>
