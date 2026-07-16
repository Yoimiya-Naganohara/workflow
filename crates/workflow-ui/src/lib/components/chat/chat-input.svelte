<script lang="ts">
	import { Button } from "$lib/components/ui/button";
	import { Textarea } from "$lib/components/ui/textarea";
	import { Send } from "@lucide/svelte";
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
		onSubmit: (e?: Event) => void;
	} = $props();

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === "Enter" && !e.shiftKey) {
			e.preventDefault();
			onSubmit(e);
		}
	}
</script>

<form onsubmit={onSubmit} class="mx-auto max-w-3xl flex gap-2 items-end">
	<div class="flex-1 relative">
		<Textarea
			bind:value
			class="min-h-9 max-h-32 resize-none text-sm pr-9"
			placeholder={disabled ? "Select an agent first..." : "Type a message..."}
			{disabled}
			onkeydown={handleKeydown}
		/>
		<div class="absolute right-1 bottom-1">
			<Button
				type="submit"
				disabled={!value.trim() || disabled || pendingAction?.type === "send"}
				size="icon-sm"
			>
				<Send class="size-3.5" />
			</Button>
		</div>
	</div>
</form>
