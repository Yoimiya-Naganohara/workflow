<script lang="ts">
	import { cn } from "$lib/utils.js";
	import { Button } from "$lib/components/ui/button";
	import { Plus, Mic, ArrowUp, Square } from "@lucide/svelte";
	import type { PendingAction } from "$lib/types";

	let {
		value = $bindable(""),
		disabled,
		pendingAction,
		onSubmit,
		showStop = false,
		onStop,
	}: {
		value?: string;
		disabled: boolean;
		pendingAction: PendingAction;
		onSubmit: () => void;
		showStop?: boolean;
		onStop?: () => void;
	} = $props();

	let focused = $state(false);

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === "Enter" && !e.shiftKey) {
			e.preventDefault();
			onSubmit();
		}
	}

	function handleTextareaInput() {
		// IME composition handling: don't send during composition
	}
</script>

<form
	onsubmit={(e) => {
		e.preventDefault();
		onSubmit();
	}}
	class={cn(
		"mx-auto max-w-3xl rounded-2xl border shadow-xs transition-all duration-200",
		"bg-card border-border/50",
		"focus-within:shadow-lg focus-within:border-border/80",
		focused && "bg-[var(--acrylic-bg)]",
	)}
>
	<textarea
		bind:value
		class={cn(
			"w-full resize-none bg-transparent px-4 pt-4 pb-1 text-sm outline-none",
			"placeholder:text-muted-foreground transition-[placeholder-color] duration-200",
			"field-sizing-content min-h-[24px] max-h-40",
		)}
		placeholder={disabled ? "Select an agent first..." : "Type a message..."}
		{disabled}
		onkeydown={handleKeydown}
		onfocus={() => (focused = true)}
		onblur={() => (focused = false)}
		oncompositionstart={handleTextareaInput}
		oncompositionend={handleTextareaInput}
		rows={1}
	></textarea>

	<div class="flex items-center justify-between px-2 pb-2.5">
		<div class="flex items-center gap-0.5">
			<Button
				variant="ghost"
				size="icon-xs"
				type="button"
				disabled={disabled}
				class="rounded-full"
			>
				<Plus class="size-4" />
			</Button>
		</div>

		<div class="flex items-center gap-0.5">
			<Button
				variant="ghost"
				size="icon-xs"
				type="button"
				disabled={disabled}
				class="rounded-full hover:bg-muted-foreground/10"
			>
				<Mic class="size-4" />
			</Button>
			{#if showStop}
				<Button
					type="button"
					size="icon-xs"
					onclick={onStop}
					class="rounded-full bg-destructive text-destructive-foreground hover:bg-destructive/90 shadow-xs"
				>
					<Square class="size-4" />
				</Button>
			{:else if value.trim() && !disabled && pendingAction?.type !== "send"}
				<Button
					type="submit"
					size="icon-xs"
					class="rounded-full bg-foreground text-background hover:bg-foreground/90 shadow-xs"
				>
					<ArrowUp class="size-4" />
				</Button>
			{:else}
				<Button
					type="submit"
					size="icon-xs"
					disabled
					class="rounded-full bg-muted text-muted-foreground/50"
				>
					<ArrowUp class="size-4" />
				</Button>
			{/if}
		</div>
	</div>
</form>
