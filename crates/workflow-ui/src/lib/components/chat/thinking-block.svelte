<script lang="ts">
	let { text }: { text: string } = $props();
</script>

<div class="flex items-start gap-2 text-xs text-muted-foreground/60">
	<div class="size-1.5 rounded-full bg-muted-foreground/30 mt-[3px] shrink-0"></div>
	<span class="leading-relaxed whitespace-pre-wrap">{text}</span>
</div>
