<script lang="ts">
	import VirtualList from "@humanspeak/svelte-virtual-list";
	import TextBlock from "$lib/components/chat/text-block.svelte";
	import ThinkingBlock from "$lib/components/chat/thinking-block.svelte";
	import ToolCard from "$lib/components/chat/tool-card.svelte";
	import ErrorBlock from "$lib/components/chat/error-block.svelte";
	import { MessageSquare } from "@lucide/svelte";
	import { Card } from "$lib/components/ui/card";
	import type { ChatItem } from "$lib/types";

	const dotColors: Record<string, string> = {
		user: "bg-primary/60",
		assistant: "bg-emerald-500",
		thinking: "bg-amber-500",
		tool: "bg-violet-500",
		error: "bg-destructive",
	};

	let {
		items,
		empty,
		agentId,
		agentRole,
	}: {
		items: ChatItem[];
		empty: boolean;
		agentId?: number | null;
		agentRole?: string;
	} = $props();
</script>

{#if empty}
	<div class="flex-1 flex items-center justify-center">
		<Card class="flex flex-col items-center gap-3 text-center py-12 px-8 max-w-xs border-dashed">
			<div class="size-12 rounded-full bg-muted flex items-center justify-center">
				<MessageSquare class="size-6 text-muted-foreground/30" />
			</div>
			<p class="text-sm font-medium text-muted-foreground/60">No messages yet</p>
			<p class="text-xs text-muted-foreground/40">Select an agent and send a message to begin.</p>
		</Card>
	</div>
{:else}
	<div class="flex-1 min-h-0 flex flex-col">
		{#if agentId != null}
			<div class="shrink-0 mx-auto w-full max-w-3xl px-4 sm:px-6 pt-2 pb-1">
				<div class="flex items-center gap-2 text-[10px] text-muted-foreground/40">
					<span class="font-mono">#{agentId}</span>
					<span class="w-px h-3 bg-border/30"></span>
					<span>{agentRole ?? "agent"}</span>
					<span class="ml-auto tabular-nums">{items.length} messages</span>
				</div>
			</div>
		{/if}
		<VirtualList {items} defaultEstimatedItemHeight={60}>
			{#snippet renderItem(item: ChatItem, index: number)}
				<div class="mx-auto max-w-3xl px-4 sm:px-6">
					<div class="flex gap-3">
						<div class="flex flex-col items-center shrink-0 pt-[18px]">
							<div class="size-2 rounded-full {dotColors[item.type] ?? 'bg-muted-foreground/30'} ring-2 ring-background"></div>
							{#if index < items.length - 1}
								<div class="w-px flex-1 min-h-4 bg-border/40 mt-1"></div>
							{/if}
						</div>
						<div class="flex-1 min-w-0 pb-1">
							{#if item.type === "assistant"}
								<TextBlock text={item.text} html={item.html ?? ""} role="assistant" streaming={item.streaming ?? false} />
							{:else if item.type === "user"}
								<TextBlock text={item.text} html={item.html ?? ""} role="user" />
							{:else if item.type === "thinking"}
								<ThinkingBlock text={item.text} />
							{:else if item.type === "tool"}
								<ToolCard name={item.text} result={item.result ?? undefined} status={item.status ?? "done"} />
							{:else if item.type === "error"}
								<ErrorBlock text={item.text} />
							{/if}
						</div>
					</div>
				</div>
			{/snippet}
		</VirtualList>
	</div>
{/if}
