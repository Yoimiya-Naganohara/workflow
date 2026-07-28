<script lang="ts">
	import { Button } from "$lib/components/ui/button";
	import { Input } from "$lib/components/ui/input";
	import { Loader2 } from "@lucide/svelte";
	import { cn } from "$lib/utils.js";
	import type { McpServerConfig, McpTransport } from "$lib/types";

	let {
		adding,
		onAdd,
		onCancel,
	}: {
		adding: boolean;
		onAdd: (config: McpServerConfig) => void;
		onCancel: () => void;
	} = $props();

	let name = $state("");
	let transport = $state<"stdio" | "sse" | "streamable_http">("stdio");
	let command = $state("");
	let args = $state("");
	let url = $state("");
	let dangerousTools = $state("");

	function reset() {
		name = "";
		transport = "stdio";
		command = "";
		args = "";
		url = "";
		dangerousTools = "";
	}

	async function handleAdd() {
		if (!name.trim()) return;
		let t: McpTransport;
		if (transport === "stdio") {
			const a = args.split(",").map((s) => s.trim()).filter((s) => s.length > 0);
			t = { type: "stdio", command: command.trim(), args: a };
		} else if (transport === "sse") {
			t = { type: "sse", url: url.trim() };
		} else {
			t = { type: "streamable_http", url: url.trim() };
		}
		const config: McpServerConfig = { name: name.trim(), transport: t };
		const dangerous = dangerousTools.split(",").map((s) => s.trim()).filter((s) => s.length > 0);
		if (dangerous.length > 0) config.dangerous_tools = dangerous;
		await onAdd(config);
		reset();
	}

	function handleCancel() {
		reset();
		onCancel();
	}
</script>

<div class="flex flex-col gap-2 px-2 py-2 mt-1 rounded-md bg-muted/30 border border-border/30">
	<span class="text-[11px] font-semibold text-muted-foreground uppercase tracking-wider">New Server</span>

	<div class="flex flex-col gap-1">
		<label for="mcp-name" class="text-[10px] text-muted-foreground/70">Name</label>
		<Input id="mcp-name" type="text" placeholder="my-server" class="h-6 text-xs px-2" bind:value={name} />
	</div>

	<div class="flex flex-col gap-1">
		<span class="text-[10px] text-muted-foreground/70">Transport</span>
		<div class="flex gap-1">
			{#each ["stdio", "sse", "streamable_http"] as t}
				<button
					class={cn(
						"flex-1 text-[10px] px-1.5 py-1 rounded font-medium transition-colors",
						transport === t ? "bg-accent text-accent-foreground" : "bg-muted/50 text-muted-foreground/70 hover:bg-muted",
					)}
					onclick={() => { transport = t as typeof transport; }}
				>
					{t}
				</button>
			{/each}
		</div>
	</div>

	{#if transport === "stdio"}
		<div class="flex flex-col gap-1">
			<label for="mcp-command" class="text-[10px] text-muted-foreground/70">Command</label>
			<Input id="mcp-command" type="text" placeholder="npx, uvx, node..." class="h-6 text-xs px-2" bind:value={command} />
		</div>
		<div class="flex flex-col gap-1">
			<label for="mcp-args" class="text-[10px] text-muted-foreground/70">Args (comma-separated)</label>
			<Input id="mcp-args" type="text" placeholder="-y, @modelcontextprotocol/server-filesystem, /path" class="h-6 text-xs px-2" bind:value={args} />
		</div>
	{:else}
		<div class="flex flex-col gap-1">
			<label for="mcp-url" class="text-[10px] text-muted-foreground/70">URL</label>
			<Input id="mcp-url" type="text" placeholder="https://..." class="h-6 text-xs px-2" bind:value={url} />
		</div>
	{/if}

	<div class="flex flex-col gap-1">
		<label for="mcp-dangerous" class="text-[10px] text-muted-foreground/70">
			Dangerous tools <span class="text-muted-foreground/40">(optional, comma-separated, or * for all)</span>
		</label>
		<Input id="mcp-dangerous" type="text" placeholder="*" class="h-6 text-xs px-2" bind:value={dangerousTools} />
	</div>

	<div class="flex items-center gap-1.5 mt-1">
		<Button
			variant="default"
			size="xs"
			class="flex-1 h-6 text-[11px]"
			disabled={adding || !name.trim() || (transport === "stdio" && !command.trim())}
			onclick={handleAdd}
		>
			{#if adding}<Loader2 class="size-2.5 animate-spin" />{/if}
			Add
		</Button>
		<Button variant="ghost" size="xs" class="h-6 text-[11px]" onclick={handleCancel}>Cancel</Button>
	</div>
</div>
