<script lang="ts">
	import { cn } from "$lib/utils.js";

	let {
		class: className,
		...rest
	}: { class?: string } & import("svelte/elements").HTMLAttributes<HTMLDivElement> = $props();
</script>

<div
	class={cn(
		"animate-pulse rounded-md bg-muted-foreground/10",
		className
	)}
	{...rest}
></div>
