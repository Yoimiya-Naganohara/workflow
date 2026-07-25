<script lang="ts">
	import { Button } from "$lib/components/ui/button";
	import type { Snippet } from "svelte";

	let {
		icon,
		title,
		description,
		action,
		actionLabel,
		onAction,
		secondaryAction,
		secondaryLabel,
		onSecondaryAction,
	}: {
		icon?: Snippet;
		title: string;
		description?: string;
		action?: Snippet;
		actionLabel?: string;
		onAction?: () => void;
		secondaryAction?: Snippet;
		secondaryLabel?: string;
		onSecondaryAction?: () => void;
	} = $props();
</script>

<div class="flex-1 flex items-center justify-center">
	<div class="flex flex-col items-center gap-3 text-center py-12 px-8 max-w-xs">
		{#if icon}
			<div class="size-12 rounded-full bg-muted/60 flex items-center justify-center">
				{@render icon()}
			</div>
		{/if}
		<p class="text-sm font-medium text-muted-foreground/70">{title}</p>
		{#if description}
			<p class="text-xs text-muted-foreground/40 leading-relaxed">{description}</p>
		{/if}
		<div class="flex items-center gap-2 mt-1">
			{#if actionLabel && onAction}
				<Button variant="outline" size="xs" onclick={onAction}>
					{actionLabel}
				</Button>
			{/if}
			{#if action}
				{@render action()}
			{/if}
			{#if secondaryLabel && onSecondaryAction}
				<Button variant="ghost" size="xs" onclick={onSecondaryAction}>
					{secondaryLabel}
				</Button>
			{/if}
			{#if secondaryAction}
				{@render secondaryAction()}
			{/if}
		</div>
	</div>
</div>
