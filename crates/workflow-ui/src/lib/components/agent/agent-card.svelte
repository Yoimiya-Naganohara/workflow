<script lang="ts">
	import { cn } from "$lib/utils.js";
	import { Button } from "$lib/components/ui/button";
	import Badge from "$lib/components/ui/badge/badge.svelte";
	import { Trash2 } from "@lucide/svelte";
	import type { AgentInfo, AgentId } from "$lib/types";

	let {
		agent,
		selected,
		status,
		onSelect,
		onRemove,
	}: {
		agent: AgentInfo;
		selected: boolean;
		status: "idle" | "thinking" | "running-tool" | "responding" | "error";
		onSelect: (id: AgentId) => void;
		onRemove: (id: AgentId) => void;
	} = $props();

	const statusColor = $derived.by(() => {
		switch (status) {
			case "thinking": return "bg-amber-500 animate-pulse";
			case "running-tool": return "bg-amber-500 animate-pulse";
			case "responding": return "bg-emerald-500";
			case "error": return "bg-destructive";
			case "idle": return "bg-muted-foreground/40";
		}
	});

	const statusLabel = $derived.by(() => {
		switch (status) {
			case "thinking": return "thinking";
			case "running-tool": return "running tool";
			case "responding": return "replying";
			case "error": return "error";
			case "idle": return "idle";
		}
	});
</script>

<div class="flex items-center gap-0.5 group">
	<Button
		variant="ghost"
		class={cn(
			"w-full justify-start gap-2 px-2 py-1.5 text-xs font-medium rounded-md",
			selected && "bg-accent text-accent-foreground border-l-2 border-primary rounded-s-none"
		)}
		onclick={() => onSelect(agent.id)}
	>
		<div class="relative shrink-0">
			<div class={cn("size-1.5 rounded-full", statusColor)}></div>
		</div>
		<div class="flex items-center gap-1.5 min-w-0 flex-1">
			<span class="truncate font-mono text-[11px]">#{agent.id}</span>
			<Badge variant="outline" class="text-[10px] px-1 py-px font-normal leading-none shrink-0">{agent.role}</Badge>
		</div>
		{#if agent.current_task}
			<span class="text-[10px] text-muted-foreground/50 truncate max-w-[80px] hidden group-hover:block" title={agent.current_task}>
				{agent.current_task}
			</span>
		{/if}
	</Button>
	<Button
		variant="ghost"
		size="icon-xs"
		class="opacity-0 group-hover:opacity-100 shrink-0 hover:bg-destructive/10 text-muted-foreground hover:text-destructive transition-all duration-150"
		onclick={() => onRemove(agent.id)}
	>
		<Trash2 class="size-3" />
	</Button>
</div>
