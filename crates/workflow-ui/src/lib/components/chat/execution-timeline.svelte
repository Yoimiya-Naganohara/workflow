<script lang="ts">
	import TextBlock from "$lib/components/chat/text-block.svelte";
	import ThinkingBlock from "$lib/components/chat/thinking-block.svelte";
	import ToolCard from "$lib/components/chat/tool-card.svelte";
	import ErrorBlock from "$lib/components/chat/error-block.svelte";
	import { MessageSquare, Pin, PinOff } from "@lucide/svelte";
	import { Card } from "$lib/components/ui/card";
	import { Button } from "$lib/components/ui/button";
	import type { ChatItem, PinnedMessage } from "$lib/types";

	let {
		items,
		empty,
		agentRole,
		pinnedMessages,
		onPin,
	}: {
		items: ChatItem[];
		empty: boolean;
		agentRole?: string;
		pinnedMessages: PinnedMessage[];
		onPin: (item: ChatItem) => void;
	} = $props();

	function isPinned(item: ChatItem): boolean {
		return pinnedMessages.some(p => p.chatItemId === item.id);
	}

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

	let prevLen = 0;
	let prevLastId: number | null = null;

	$effect(() => {
		const len = items.length;
		const last = items[len - 1];
		if (!scrollContainer || userScrolledUp) return;

		if (len > prevLen || (last && last.id !== prevLastId)) {
			queueMicrotask(() => scrollToBottom());
		}

		prevLen = len;
		prevLastId = last?.id ?? null;
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
		{#if agentRole}
			<div class="shrink-0 mx-auto w-full max-w-3xl px-4 sm:px-6 pt-3 pb-0">
				<div class="text-[10px] text-muted-foreground/40 font-medium">{agentRole}</div>
			</div>
		{/if}
		<div
			role="log"
			aria-live="polite"
			aria-label="Chat messages"
			bind:this={scrollContainer}
			class="flex-1 overflow-y-auto"
			onscroll={onScroll}
		>
			<div class="mx-auto max-w-3xl px-4 sm:px-6 py-4 space-y-3">
				{#each items as item (item.id)}
					{@const pinned = isPinned(item)}
					<div class="group relative">
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
						<div
							class={`absolute top-0 right-0 transition-opacity ${pinned ? 'opacity-100' : 'opacity-0 group-hover:opacity-100'}`}
						>
							<Button
								variant="ghost"
								size="icon-xs"
								class={pinned ? 'text-amber-500 hover:text-amber-600' : 'text-muted-foreground/40 hover:text-foreground'}
								onclick={() => onPin(item)}
								title={pinned ? "Unpin message" : "Pin to sidebar"}
								aria-label={pinned ? "Unpin message" : "Pin to sidebar"}
							>
								{#if pinned}
									<PinOff class="size-3" />
								{:else}
									<Pin class="size-3" />
								{/if}
							</Button>
						</div>
					</div>
				{/each}
			</div>
		</div>
	</div>
{/if}
