<script lang="ts">
	import { Button } from "$lib/components/ui/button";
	import type { Snippet } from "svelte";

	let {
		icon,
		onclick,
		title,
		size = "icon-xs",
		variant = "ghost",
		class: className,
		disabled = false,
		...rest
	}: {
		icon: Snippet;
		onclick: (e: MouseEvent) => void;
		title: string;
		size?: "icon-xs" | "icon-sm";
		variant?: "ghost" | "outline" | "default";
		class?: string;
		disabled?: boolean;
		[k: string]: unknown;
	} = $props();
</script>

<Button {variant} {size} {onclick} {disabled} title={title} aria-label={title} class={className} {...rest}>
	{@render icon()}
</Button>
