<script lang="ts">
	import VirtualList from "@humanspeak/svelte-virtual-list";
	import TextBlock from "$lib/components/chat/text-block.svelte";
	import ThinkingBlock from "$lib/components/chat/thinking-block.svelte";
	import ToolCard from "$lib/components/chat/tool-card.svelte";
	import ErrorBlock from "$lib/components/chat/error-block.svelte";
	import { ChevronDown, MessageSquare } from "@lucide/svelte";
	import { Card } from "$lib/components/ui/card";
	import type { ChatItem } from "$lib/types";

	const dotColors: Record<string, string> = {
		user: "bg-primary/60",
		assistant: "bg-emerald-500",
		thinking: "bg-amber-500",
		tool: "bg-violet-500",
		error: "bg-destructive",
	};

	const typeLabels: Record<string, string> = {
		thinking: "Thinking",
		tool: "Tool call",
		error: "Error",
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

	let scrollContainer = $state<HTMLDivElement | null>(null);
	let userScrolledUp = $state(false);
	const SCROLL_THRESHOLD = 60;

	function isNearBottom(el: HTMLDivElement): boolean {
		return el.scrollHeight - el.scrollTop - el.clientHeight < SCROLL_THRESHOLD;
	}

	function scrollToBottom() {
		if (!scrollContainer) return;
		scrollContainer.scrollTop = scrollContainer.scrollHeight;
		userScrolledUp = false;
	}

	function onScroll() {
		if (!scrollContainer) return;
		userScrolledUp = !isNearBottom(scrollContainer);
	}

	let prevLen = $state(0);

	$effect(() => {
		const len = items.length;
		if (len > prevLen && scrollContainer && !userScrolledUp) {
			// Use microtask to let the DOM update first
			queueMicrotask(() => scrollToBottom());
		}
		prevLen = len;
	});

	// Also scroll on streaming updates (last item text changes)
	let prevLastId = $state<number | null>(null);
	$effect(() => {
		const last = items[items.length - 1];
		if (!last || !scrollContainer) return;
		if (last.id !== prevLastId) {
			prevLastId = last.id;
			return;
		}
		// Same last item — streaming update
		if (!userScrolledUp) {
			queueMicrotask(() => scrollToBottom());
		}
	});
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
		<div role="log" aria-live="polite" aria-label="Chat messages" class="contents">
<div
	bind:this={scrollContainer}
	class="relative flex-1 overflow-y-auto"
	onscroll={onScroll}
>
<VirtualList {items} defaultEstimatedItemHeight={60} containerClass="size-full">
			{#snippet renderItem(item: ChatItem, index: number)}
				<div class="mx-auto max-w-3xl px-4 sm:px-6">
					<div class="flex gap-3">
						<div class="flex flex-col items-center shrink-0 pt-[18px]">
							<div class="size-2 rounded-full {dotColors[item.type] ?? 'bg-muted-foreground/30'} ring-2 ring-background {item.status === 'running' ? 'animate-pulse' : ''}"></div>
							{#if index < items.length - 1}
								<div class="w-px flex-1 min-h-4 bg-border/40 mt-1 {index === items.length - 2 ? 'bg-gradient-to-b from-border/40 to-transparent' : ''}"></div>
							{/if}
						</div>
						<div class="flex-1 min-w-0 pb-1">
							{#key item.id}
								<div class="animate-in" style="animation-delay: 0ms">
									{#if item.type === "assistant"}
										<TextBlock text={item.text} role="assistant" streaming={item.streaming ?? false} />
									{:else if item.type === "user"}
										<TextBlock text={item.text} role="user" />
									{:else if item.type === "thinking"}
										<ThinkingBlock text={item.text} />
									{:else if item.type === "tool"}
										<ToolCard name={item.text} result={item.result == null ? undefined : item.result} status={item.status ?? "done"} />
									{:else if item.type === "error"}
										<ErrorBlock text={item.text} />
									{/if}
								</div>
							{/key}
						</div>
					</div>
				</div>
			{/snippet}
		</VirtualList>
	{#if userScrolledUp}
		<button
			onclick={scrollToBottom}
			class="absolute bottom-3 right-4 z-10 size-8 rounded-full bg-background border border-border shadow-md flex items-center justify-center hover:bg-accent transition-colors animate-in"
			aria-label="Scroll to bottom"
		>
			<ChevronDown class="size-4 text-muted-foreground" />
		</button>
	{/if}
</div>
		</div>
	</div>
{/if}
