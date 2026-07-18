<script lang="ts">
	import { Zap, Search, Globe, FileText, Terminal, CheckCircle2, AlertTriangle, Copy, ChevronRight } from "@lucide/svelte";

	let {
		name: raw,
		result,
		status,
	}: { name: string; result?: string; status: "running" | "done" } = $props();
	let expanded = $state(false);

	let toolName = $derived(raw.split(": ")[0]);
	let toolArgs = $derived(raw.slice(toolName.length + 2));

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

	let isError = $derived(!!(result && (result.startsWith("error:") || result.startsWith("Error:") || result.startsWith("failed:") || result.startsWith("Failed:"))));

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
		let text = result;
		try {
			const parsed = JSON.parse(result);
			text = JSON.stringify(parsed);
		} catch { /* not JSON, use raw */ }
		const line = text.split("\n")[0].trim();
		return line.length > 90 ? line.slice(0, 90) + "…" : line;
	});

	let formattedArgs = $derived.by(() => {
		if (!toolArgs) return null;
		try {
			const parsed = JSON.parse(toolArgs);
			return JSON.stringify(parsed, null, 2);
		} catch {
			return toolArgs;
		}
	});

	let Icon = $derived(iconForTool(toolName));

	let startTime = $state(0);
	let elapsed = $state("");

	$effect(() => {
		if (status === "running") {
			startTime = Date.now();
			elapsed = "";
		}
	});

	$effect(() => {
		if (status === "done" && startTime > 0) {
			const ms = Date.now() - startTime;
			if (ms < 1000) {
				elapsed = `${ms}ms`;
			} else {
				elapsed = `${(ms / 1000).toFixed(1)}s`;
			}
		}
	});

	let copyButtonText = $state("Copy");
	function copyResult() {
		if (!result) return;
		navigator.clipboard.writeText(result);
		copyButtonText = "Copied!";
		setTimeout(() => { copyButtonText = "Copy"; }, 2000);
	}
</script>

<div
	class="rounded-lg border {status === 'running' ? 'border-amber-500/20 bg-amber-500/[3%]' : isError ? 'border-red-500/20 bg-red-500/[3%]' : 'border-border/60 bg-muted/30'} transition-all duration-200"
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
			{:else if isError}
				<div class="size-5 rounded-full bg-red-500/10 flex items-center justify-center">
					<AlertTriangle class="size-3.5 text-red-500" />
				</div>
			{:else}
				<div class="size-5 rounded-full bg-emerald-500/10 flex items-center justify-center">
					<CheckCircle2 class="size-3.5 text-emerald-500" />
				</div>
			{/if}
		</div>
		<div class="flex items-center gap-1.5 min-w-0">
			<Icon class="size-3.5 text-muted-foreground/60 shrink-0" />
			<span class="text-xs font-medium text-foreground/80 truncate">{formatName(toolName)}</span>
		</div>
		<div class="flex-1 min-w-0"></div>
		{#if status === "running"}
			<span class="text-[10px] font-medium text-amber-600 dark:text-amber-400 shrink-0 flex items-center gap-1">
				<span class="size-1 rounded-full bg-amber-500 animate-pulse"></span>
				running
			</span>
		{:else}
			{#if elapsed}
				<span class="text-[10px] text-muted-foreground/40 shrink-0 tabular-nums">{elapsed}</span>
			{/if}
			{#if result && !expanded}
				<span class="text-[10px] text-muted-foreground/50 font-mono truncate max-w-[120px]">{preview}</span>
			{/if}
		{/if}
		<ChevronRight
			class="size-3 shrink-0 text-muted-foreground/40 transition-transform duration-200 {expanded ? 'rotate-90' : ''}"
		/>
	</button>

	{#if expanded && (toolArgs || result)}
		<div class="px-3 pb-2.5 pt-0 space-y-2 animate-fade-in">
			{#if toolArgs}
				<div>
					<p class="text-[10px] font-medium text-muted-foreground/50 uppercase tracking-wider mb-1">Arguments</p>
					<pre class="text-xs text-muted-foreground/70 bg-muted-foreground/5 rounded-lg p-2 overflow-x-auto whitespace-pre-wrap font-mono leading-relaxed">{formattedArgs ?? toolArgs}</pre>
				</div>
			{/if}
			{#if result}
				<div>
					<div class="flex items-center justify-between mb-1">
						<p class="text-[10px] font-medium text-muted-foreground/50 uppercase tracking-wider">Result</p>
						<button
							onclick={(e) => { e.stopPropagation(); copyResult(); }}
							class="flex items-center gap-1 text-[10px] text-muted-foreground/50 hover:text-foreground/70 transition-colors"
						>
							<Copy class="size-3" />
							{copyButtonText}
						</button>
					</div>
					<pre class="text-xs {isError ? 'text-red-500/80' : 'text-muted-foreground/70'} bg-muted-foreground/5 rounded-lg p-2.5 overflow-x-auto whitespace-pre-wrap font-mono leading-relaxed">{formattedResult ?? result}</pre>
				</div>
			{/if}
		</div>
	{/if}
</div>