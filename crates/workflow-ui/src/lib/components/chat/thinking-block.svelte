<script lang="ts">
	let { text }: { text: string } = $props();
</script>

<div class="flex items-start gap-2 text-xs text-muted-foreground/60 animate-in">
	<span class="leading-relaxed whitespace-pre-wrap italic">{text}</span>
</div>
