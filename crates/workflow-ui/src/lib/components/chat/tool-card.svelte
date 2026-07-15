<script lang="ts">
	import { Zap, Search, Globe, FileText, Terminal, CheckCircle2 } from "@lucide/svelte";

	let { name, result, status }: { name: string; result?: string; status: "running" | "done" } = $props();
	let expanded = $state(false);

	function formatName(n: string) {
		return n.replace(/[_-]/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
	}

	function iconForTool(n: string) {
		const s = n.toLowerCase();
		if (s.includes("search")) return Search;
		if (s.includes("web") || s.includes("http") || s.includes("fetch") || s.includes("url")) return Globe;
		if (s.includes("file") || s.includes("read") || s.includes("write") || s.includes("mv") || s.includes("cp")) return FileText;
		if (s.includes("bash") || s.includes("sh ") || s.includes("exec") || s.includes("run") || s.includes("code") || s.includes("shell") || s.includes("terminal")) return Terminal;
		return Zap;
	}

	let formattedResult = $derived.by(() => {
		if (!result) return null;
		try {
			const parsed = JSON.parse(result);
			return JSON.stringify(parsed, null, 2);
		} catch {
			return result;
		}
	});

	let preview = $derived.by(() => {
		if (!result) return "";
		const line = result.split("\n")[0].trim();
		return line.length > 90 ? line.slice(0, 90) + "…" : line;
	});

	let Icon = $derived(iconForTool(name));
</script>

<div
	class="rounded-lg border {status === 'running' ? 'border-amber-500/20 bg-amber-500/[3%]' : 'border-border/60 bg-muted/30'} transition-all duration-200"
>
	<button
		onclick={() => expanded = !expanded}
		class="flex items-center gap-2.5 w-full px-3 py-2 text-left group"
	>
		<div class="relative size-5 shrink-0 flex items-center justify-center">
			{#if status === "running"}
				<div class="size-5 rounded-full bg-amber-500/15 flex items-center justify-center">
					<span class="size-2 rounded-full bg-amber-500 animate-ping absolute"></span>
					<span class="size-2 rounded-full bg-amber-500 relative"></span>
				</div>
			{:else}
				<div class="size-5 rounded-full bg-emerald-500/10 flex items-center justify-center">
					<CheckCircle2 class="size-3.5 text-emerald-500" />
				</div>
			{/if}
		</div>
		<span class="text-xs font-medium text-foreground/80 truncate">{formatName(name)}</span>
		<div class="flex-1 min-w-0"></div>
		{#if status === "running"}
			<span class="text-[10px] font-medium text-amber-600 dark:text-amber-400 shrink-0 flex items-center gap-1">
				<span class="size-1 rounded-full bg-amber-500 animate-pulse"></span>
				running
			</span>
		{:else if result && !expanded}
			<span class="text-[10px] text-muted-foreground/50 font-mono truncate max-w-[120px]">{preview}</span>
		{/if}
		<svg
			class="size-3 shrink-0 text-muted-foreground/40 transition-transform duration-200 {expanded ? 'rotate-90' : ''}"
			viewBox="0 0 24 24"
			fill="none"
			stroke="currentColor"
			stroke-width="2"
		><path d="m9 18 6-6-6-6"/></svg>
	</button>
	{#if expanded && result}
		<div class="px-3 pb-2.5 pt-0 animate-fade-in">
			<pre class="text-xs text-muted-foreground/70 bg-muted-foreground/5 rounded-lg p-2.5 overflow-x-auto whitespace-pre-wrap font-mono leading-relaxed">{formattedResult ?? result}</pre>
		</div>
	{/if}
</div>
