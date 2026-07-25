<script lang="ts">
	import { cn } from "$lib/utils.js";

	let { class: className, ...restProps }: { class?: string } & Record<string, unknown> = $props();
</script>

<hr class={cn("bg-border -mx-1 my-1 h-px pointer-events-none", className)} {...restProps} />
