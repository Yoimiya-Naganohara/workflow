<script lang="ts">
	import { cn } from "$lib/utils.js";
	import { Button } from "$lib/components/ui/button";
	import { Plus, Mic, ArrowUp } from "@lucide/svelte";
	import type { PendingAction } from "$lib/types";

	let {
		value = $bindable(""),
		disabled,
		pendingAction,
		onSubmit,
	}: {
		value?: string;
		disabled: boolean;
		pendingAction: PendingAction;
		onSubmit: () => void;
	} = $props();

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === "Enter" && !e.shiftKey) {
			e.preventDefault();
			onSubmit();
		}
	}
</script>

<form
	onsubmit={(e) => {
		e.preventDefault();
		onSubmit();
	}}
	class={cn(
		"mx-auto max-w-3xl rounded-2xl border shadow-xs transition-all duration-200 focus-within:shadow-md bg-card focus-within:bg-background focus-within:border-border border-border/50",
	)}>
	<textarea
		bind:value
		class="w-full resize-none bg-transparent px-4 pt-3.5 pb-1 text-sm outline-none placeholder:text-muted-foreground field-sizing-content min-h-[22px] max-h-40"
		placeholder={disabled ? "Select an agent first..." : "Type a message..."}
		{disabled}
		onkeydown={handleKeydown}
		rows={1}
	></textarea>

	<div class="flex items-center justify-between px-2 pb-2.5">
		<div class="flex items-center gap-0.5">
			<Button variant="ghost" size="icon-xs" type="button" disabled={disabled}>
				<Plus class="size-4" />
			</Button>
		</div>

		<div class="flex items-center gap-0.5">
			<Button variant="ghost" size="icon-xs" type="button" disabled={disabled}>
				<Mic class="size-4" />
			</Button>
			<Button
				type="submit"
				size="icon-xs"
				disabled={!value.trim() || disabled || pendingAction?.type === "send"}
				class="rounded-full"
			>
				<ArrowUp class="size-4" />
			</Button>
		</div>
	</div>
</form>
