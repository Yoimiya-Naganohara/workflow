<script lang="ts">
	import ExecutionTimeline from "$lib/components/chat/execution-timeline.svelte";
	import ChatInput from "$lib/components/chat/chat-input.svelte";
	import type {
		AgentId,
		AgentInfo,
		ChatItem,
		PendingAction,
		PinnedMessage,
	} from "$lib/types";

	let {
		empty = true,
		chatItems,
		selected,
		agents,
		error,
		input = $bindable(""),
		pinnedMessages,
		pendingAction,
		running,
		onPin,
		onDismissError,
		onSubmit,
		onStop,
	}: {
		empty?: boolean;
		chatItems: ChatItem[];
		selected: AgentId | null;
		agents: AgentInfo[];
		error: string | null;
		input?: string;
		pinnedMessages: PinnedMessage[];
		pendingAction: PendingAction;
		running: boolean;
		onPin: (item: ChatItem) => void;
		onDismissError: () => void;
		onSubmit: () => void;
		onStop: () => void;
	} = $props();
</script>

<div
	class="flex flex-col flex-1 min-w-0 min-h-0"
	class:justify-center={empty}
>
	{#if !empty}
		<ExecutionTimeline
			items={chatItems}
			empty={false}
			agentRole={agents.find(a => a.id === selected)?.role}
			pinnedMessages={pinnedMessages}
			onPin={onPin}
		/>
	{/if}

	{#if error}
		<div class="animate-in shrink-0 mx-3 mb-2">
			<div class="flex items-start gap-2.5 rounded-lg bg-destructive/8 border border-destructive/20 px-3 py-2">
				<svg
					xmlns="http://www.w3.org/2000/svg"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
					stroke-width="2"
					stroke-linecap="round"
					stroke-linejoin="round"
					class="size-3.5 shrink-0 mt-0.5 text-destructive"
				><path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3" /><path d="M12 9v4" /><path d="M12 17h.01" /></svg>
				<p class="flex-1 text-xs text-destructive leading-relaxed">
					{error}
				</p>
				<button
					onclick={onDismissError}
					class="shrink-0 mt-0.5 text-destructive/50 hover:text-destructive transition-colors"
					aria-label="Dismiss error"
				>
					<svg
						xmlns="http://www.w3.org/2000/svg"
						viewBox="0 0 24 24"
						fill="none"
						stroke="currentColor"
						stroke-width="2"
						stroke-linecap="round"
						stroke-linejoin="round"
						class="size-3"
					><path d="M18 6 6 18" /><path d="m6 6 12 12" /></svg>
				</button>
			</div>
		</div>
	{/if}

	<div class="shrink-0 px-3 pb-3">
		<ChatInput
			bind:value={input}
			disabled={selected == null}
			pendingAction={pendingAction}
			showStop={running}
			onSubmit={onSubmit}
			onStop={onStop}
		/>
	</div>
</div>
