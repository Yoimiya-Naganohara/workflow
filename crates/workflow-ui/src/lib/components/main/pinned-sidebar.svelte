<script lang="ts">
	import PinnedPanel from "$lib/components/agent/pinned-panel.svelte";
	import ResizeHandle from "./resize-handle.svelte";
	import type { PinnedMessage } from "$lib/types";

	let {
		messages,
		panelWidth,
		showResizeHandle,
		onUnpin,
		onResize,
	}: {
		messages: PinnedMessage[];
		panelWidth: number;
		showResizeHandle: boolean;
		onUnpin: (pinId: number) => void;
		onResize: (deltaX: number) => void;
	} = $props();
</script>

<div
	class="relative shrink-0 bg-background flex flex-col min-h-0"
	style="width: {panelWidth}px"
>
	{#if showResizeHandle}
		<ResizeHandle onResize={onResize} />
	{/if}
	<PinnedPanel
		{messages}
		{onUnpin}
		empty={messages.length === 0}
	/>
</div>
