<script lang="ts">
	import { Button } from "$lib/components/ui/button";
	import { Badge } from "$lib/components/ui/badge";
	import { Input } from "$lib/components/ui/input";
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
	import type { McpConnectionInfo, McpServerConfig, McpTransport } from "$lib/types";

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

	// Add form fields
	let newName = $state("");
	let newTransport = $state<"stdio" | "sse" | "streamable_http">("stdio");
	let newCommand = $state("");
	let newArgs = $state("");
	let newUrl = $state("");
	let newDangerousTools = $state("");

	function resetForm() {
		newName = "";
		newTransport = "stdio";
		newCommand = "";
		newArgs = "";
		newUrl = "";
		newDangerousTools = "";
		showAddForm = false;
	}

	function transportLabel(t: McpTransport): string {
		if (t.type === "stdio") return `${t.command} ${t.args.join(" ")}`;
		if (t.type === "sse") return t.url;
		return t.url;
	}

	function isConnected(name: string): boolean {
		return connections.some((c) => c.name === name);
	}

	function getConnection(name: string): McpConnectionInfo | undefined {
		return connections.find((c) => c.name === name);
	}

	async function handleAdd() {
		if (!newName.trim()) return;
		adding = true;
		try {
			let transport: McpTransport;
			if (newTransport === "stdio") {
				const args = newArgs
					.split(",")
					.map((a) => a.trim())
					.filter((a) => a.length > 0);
				transport = { type: "stdio", command: newCommand.trim(), args };
			} else if (newTransport === "sse") {
				transport = { type: "sse", url: newUrl.trim() };
			} else {
				transport = { type: "streamable_http", url: newUrl.trim() };
			}
			const config: McpServerConfig = {
				name: newName.trim(),
				transport,
			};
			const dangerous = newDangerousTools
				.split(",")
				.map((t) => t.trim())
				.filter((t) => t.length > 0);
			if (dangerous.length > 0) {
				config.dangerous_tools = dangerous;
			}
			await onAdd(config);
			resetForm();
		} catch {
			// error handled by parent
		} finally {
			adding = false;
		}
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
					showAddForm = !showAddForm;
					if (!showAddForm) resetForm();
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

			<!-- Add server form -->
			{#if showAddForm}
				<div class="flex flex-col gap-2 px-2 py-2 mt-1 rounded-md bg-muted/30 border border-border/30">
					<span class="text-[11px] font-semibold text-muted-foreground uppercase tracking-wider">New Server</span>

					<div class="flex flex-col gap-1">
						<label for="mcp-name" class="text-[10px] text-muted-foreground/70">Name</label>
						<Input
							id="mcp-name"
							type="text"
							placeholder="my-server"
							class="h-6 text-xs px-2"
							bind:value={newName}
						/>
					</div>

					<div class="flex flex-col gap-1">
						<span class="text-[10px] text-muted-foreground/70">Transport</span>
						<div class="flex gap-1">
							{#each ["stdio", "sse", "streamable_http"] as t}
								<button
									class={cn(
										"flex-1 text-[10px] px-1.5 py-1 rounded font-medium transition-colors",
										newTransport === t
											? "bg-accent text-accent-foreground"
											: "bg-muted/50 text-muted-foreground/70 hover:bg-muted",
									)}
									onclick={() => { newTransport = t as typeof newTransport; }}
								>
									{t}
								</button>
							{/each}
						</div>
					</div>

					{#if newTransport === "stdio"}
						<div class="flex flex-col gap-1">
							<label for="mcp-command" class="text-[10px] text-muted-foreground/70">Command</label>
							<Input
								id="mcp-command"
								type="text"
								placeholder="npx, uvx, node..."
								class="h-6 text-xs px-2"
								bind:value={newCommand}
							/>
						</div>
						<div class="flex flex-col gap-1">
							<label for="mcp-args" class="text-[10px] text-muted-foreground/70">Args (comma-separated)</label>
							<Input
								id="mcp-args"
								type="text"
								placeholder="-y, @modelcontextprotocol/server-filesystem, /path"
								class="h-6 text-xs px-2"
								bind:value={newArgs}
							/>
						</div>
					{:else}
						<div class="flex flex-col gap-1">
							<label for="mcp-url" class="text-[10px] text-muted-foreground/70">URL</label>
							<Input
								id="mcp-url"
								type="text"
								placeholder="https://..."
								class="h-6 text-xs px-2"
								bind:value={newUrl}
							/>
						</div>
					{/if}

					<div class="flex flex-col gap-1">
						<label for="mcp-dangerous" class="text-[10px] text-muted-foreground/70">
							Dangerous tools <span class="text-muted-foreground/40">(optional, comma-separated, or * for all)</span>
						</label>
						<Input
							id="mcp-dangerous"
							type="text"
							placeholder="*"
							class="h-6 text-xs px-2"
							bind:value={newDangerousTools}
						/>
					</div>

					<div class="flex items-center gap-1.5 mt-1">
						<Button
							variant="default"
							size="xs"
							class="flex-1 h-6 text-[11px]"
							disabled={adding || !newName.trim() || (newTransport === "stdio" && !newCommand.trim())}
							onclick={handleAdd}
						>
							{#if adding}
								<Loader2 class="size-2.5 animate-spin" />
							{/if}
							Add
						</Button>
						<Button
							variant="ghost"
							size="xs"
							class="h-6 text-[11px]"
							onclick={resetForm}
						>
							Cancel
						</Button>
					</div>
				</div>
			{/if}
		</div>
	{/if}
</div>
