<script lang="ts">
	import { AlertTriangle, X } from "@lucide/svelte";

	let {
		text,
		onDismiss,
	}: {
		text: string;
		onDismiss?: () => void;
	} = $props();
</script>

<div class="flex items-start gap-2.5 rounded-lg bg-destructive/8 border border-destructive/20 px-3 py-2.5 animate-in group">
	<AlertTriangle class="size-3.5 shrink-0 mt-0.5 text-destructive" />
	<div class="flex-1 min-w-0">
		<p class="text-xs text-destructive leading-relaxed">{text}</p>
	</div>
	{#if onDismiss}
		<button
			onclick={onDismiss}
			class="shrink-0 mt-0.5 text-destructive/50 hover:text-destructive transition-colors"
			aria-label="Dismiss error"
		>
			<X class="size-3" />
		</button>
	{/if}
</div>
