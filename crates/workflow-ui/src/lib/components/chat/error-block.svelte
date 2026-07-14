<script lang="ts">
	import { X } from "@lucide/svelte";
	let { text }: { text: string } = $props();
</script>

<div class="flex items-start gap-2 rounded-lg bg-destructive/10 border border-destructive/20 px-3 py-2">
	<X class="size-3.5 shrink-0 mt-0.5 text-destructive" />
	<div class="text-xs text-destructive leading-relaxed">{text}</div>
</div>
