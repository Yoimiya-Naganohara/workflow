<script lang="ts">
	import { AlertTriangle } from "@lucide/svelte";

	let { text }: { text: string } = $props();
</script>

<div class="flex items-start gap-2">
	<AlertTriangle class="size-3 shrink-0 mt-0.5 text-destructive/60" />
	<p class="text-xs text-destructive/70 leading-relaxed">{text}</p>
</div>
