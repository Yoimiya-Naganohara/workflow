<script lang="ts">
	import type { AgentStatus } from "$lib/types";
	import { cn } from "$lib/utils.js";

	let {
		status,
		size = "sm",
		showPulse = true,
	}: {
		status: AgentStatus;
		size?: "sm" | "md" | "lg";
		showPulse?: boolean;
	} = $props();

	const sizeMap = { sm: "size-1.5", md: "size-2", lg: "size-2.5" };

	const colorMap: Record<AgentStatus, string> = {
		idle: "bg-muted-foreground/30",
		thinking: "bg-amber-500",
		"running-tool": "bg-amber-500",
		responding: "bg-emerald-500",
		error: "bg-red-500",
	};

	const isActive = $derived(status === "thinking" || status === "running-tool" || status === "responding");

	const statusLabel: Record<AgentStatus, string> = {
		idle: "Idle",
		thinking: "Thinking",
		"running-tool": "Running tool",
		responding: "Responding",
		error: "Error",
	};
</script>

<div class="relative shrink-0 inline-flex" role="status" aria-label={statusLabel[status]}>
	<div class={cn("rounded-full", sizeMap[size], colorMap[status])}></div>
	{#if isActive && showPulse}
		<div class={cn("absolute inset-0 rounded-full animate-ping opacity-75", colorMap[status])}></div>
	{/if}
</div>
