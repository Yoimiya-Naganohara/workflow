<script lang="ts">
	import { Zap } from "@lucide/svelte";

	let { name, result, status }: { name: string; result?: string; status: "running" | "done" } = $props();
	let expanded = $state(false);
</script>

<div class="rounded-lg border border-border/60 bg-muted/30 {status === 'running' ? 'border-l-amber-500/50' : 'border-l-emerald-500/50'} border-l-2">
	<button
		onclick={() => expanded = !expanded}
		class="flex items-center gap-2 w-full px-3 py-1.5 text-left"
	>
		<div class="relative size-4 shrink-0">
			<Zap class="size-4 text-muted-foreground"></Zap>
			{#if status === "running"}
				<span class="absolute -top-0.5 -right-0.5 size-2 rounded-full bg-amber-500 animate-ping"></span>
			{/if}
		</div>
		<span class="text-xs font-mono font-medium text-foreground/80 truncate">{name}</span>
		<div class="flex-1"></div>
		{#if result}
			<span class="text-[10px] text-muted-foreground/60 font-mono truncate max-w-[120px]">{result}</span>
		{/if}
		{#if status === "running"}
			<span class="text-[10px] text-amber-600 dark:text-amber-400 font-medium shrink-0">running...</span>
		{/if}
		<svg class="size-3 shrink-0 text-muted-foreground/50 transition-transform {expanded ? 'rotate-90' : ''}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m9 18 6-6-6-6"/></svg>
	</button>
	{#if expanded && result}
		<div class="px-3 pb-2 pt-0">
			<pre class="text-xs text-muted-foreground/70 bg-muted-foreground/5 rounded p-2 overflow-x-auto whitespace-pre-wrap font-mono">{result}</pre>
		</div>
	{/if}
</div>
