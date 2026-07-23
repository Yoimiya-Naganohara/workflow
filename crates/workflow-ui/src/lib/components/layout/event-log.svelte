<script lang="ts">
	import { Bug, RefreshCw } from "@lucide/svelte";
	import { Button } from "$lib/components/ui/button";
	import { ScrollArea } from "$lib/components/ui/scroll-area";
	import { state, type LogEntry } from "$lib/state.svelte.js";

	let { open }: { open: boolean } = $props();

	const MAX_ENTRIES = 200;

	function formatEvent(entry: LogEntry): string {
		const e = entry.event;
		switch (e.type) {
			case "agent_added": return `agent ${e.agent_id} added`;
			case "agent_removed": return `agent ${e.agent_id} removed`;
			case "agent_stopped": return `agent ${e.agent_id} stopped`;
			case "agent_output": return `agent ${e.agent_id} output`;
			case "transcript_changed": return `agent ${e.agent_id} transcript changed`;
			case "roles_changed": return "roles changed";
			case "resync_required": return "resync required";
			case "error": return `error: ${e.message}`;
			case "mcp_connected": return `mcp +${e.server} (${e.tool_count} tools)`;
			case "mcp_disconnected": return `mcp -${e.server}`;
		}
	}

	function eventColor(entry: LogEntry): string {
		switch (entry.event.type) {
			case "agent_added": return "text-emerald-500";
			case "agent_removed": return "text-destructive";
			case "agent_stopped": return "text-muted-foreground";
			case "agent_output": return "text-amber-500";
			case "transcript_changed": return "text-sky-500";
			case "roles_changed": return "text-violet-500";
			case "resync_required": return "text-orange-500";
			case "error": return "text-destructive font-bold";
			case "mcp_connected": return "text-cyan-500";
			case "mcp_disconnected": return "text-rose-500";
		}
	}

	function fmtTime(ts: number): string {
		const d = new Date(ts);
		return d.toLocaleTimeString("en-US", { hour12: false });
	}
</script>

{#if open}
	<div class="absolute bottom-0 right-0 w-[420px] h-72 border-t border-l border-border bg-card shadow-2xl z-50 flex flex-col">
		<div class="flex items-center justify-between px-3 py-1.5 border-b border-border shrink-0">
			<div class="flex items-center gap-1.5">
				<Bug class="size-3.5 text-muted-foreground" />
				<span class="text-xs font-medium text-muted-foreground">Event Log</span>
				<span class="text-[10px] text-muted-foreground/40 tabular-nums">({state.eventLog.length})</span>
			</div>
			<Button variant="ghost" size="icon-xs" onclick={() => state.eventLog.length = 0}>
				<RefreshCw class="size-3" />
			</Button>
		</div>
		<ScrollArea class="flex-1 p-0">
			<div class="font-mono text-[11px] leading-relaxed">
				{#each state.eventLog.slice(-MAX_ENTRIES) as entry (entry.ts + entry.event.type)}
					<div class="flex items-start gap-2 px-3 py-1 border-b border-border/20 hover:bg-muted/30">
						<span class="text-muted-foreground/40 shrink-0 w-14 tabular-nums">{fmtTime(entry.ts)}</span>
						<span class={eventColor(entry)}>{formatEvent(entry)}</span>
					</div>
				{:else}
					<div class="flex items-center justify-center h-20 text-xs text-muted-foreground/40">
						No events yet
					</div>
				{/each}
			</div>
		</ScrollArea>
	</div>
{/if}
