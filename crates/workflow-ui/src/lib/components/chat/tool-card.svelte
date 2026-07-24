<script lang="ts">
	import {
		Zap,
		Search,
		Globe,
		FileText,
		Terminal,
		LoaderCircle,
		ChevronRight,
        CircleX,
        CircleCheck,
	} from "@lucide/svelte";

	let {
		name: raw,
		result,
		status,
	}: { name: string; result?: string; status: "running" | "done" } = $props();
	let expanded = $state(false);

	let toolName = $derived(raw.split(": ")[0]);
	let toolArgs = $derived(raw.slice(toolName.length + 2));

	function iconForTool(n: string) {
		const s = n.toLowerCase();
		if (s.includes("search")) return Search;
		if (
			s.includes("web") ||
			s.includes("http") ||
			s.includes("fetch") ||
			s.includes("url")
		)
			return Globe;
		if (
			s.includes("file") ||
			s.includes("read") ||
			s.includes("write") ||
			s.includes("mv") ||
			s.includes("cp")
		)
			return FileText;
		if (
			s.includes("bash") ||
			s.includes("sh ") ||
			s.includes("exec") ||
			s.includes("run") ||
			s.includes("code") ||
			s.includes("shell") ||
			s.includes("terminal")
		)
			return Terminal;
		return Zap;
	}

	let isError = $derived(
		!!(
			result &&
			(result.startsWith("error:") ||
				result.startsWith("Error:") ||
				result.startsWith("failed:") ||
				result.startsWith("Failed:"))
		),
	);

	let formattedResult = $derived.by(() => {
		if (!result) return null;
		try {
			const parsed = JSON.parse(result);
			return JSON.stringify(parsed, null, 2);
		} catch {
			return result;
		}
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
</script>

<div class="group">
	<button
		onclick={() => (expanded = !expanded)}
		class="flex items-center gap-1.5 text-xs text-muted-foreground/70 hover:text-foreground/80 transition-colors"
	>
		<Icon class="size-3.5 shrink-0" />
		<span class="font-medium">{toolName}</span>
		{#if toolArgs}
			<span
				class="font-mono text-muted-foreground/50 truncate max-w-[200px]"
				>{toolArgs}</span
			>
		{/if}
		{#if status === "running"}
			<LoaderCircle class="size-3 animate-spin text-amber-500 shrink-0" />
		{:else if isError}
			<CircleX class="size-3 text-red-500 shrink-0" />
		{:else}
			<CircleCheck class="size-3 text-emerald-500 shrink-0" />
		{/if}
		<ChevronRight
			class="size-3 shrink-0 text-muted-foreground/30 transition-transform duration-150 {expanded
				? 'rotate-90'
				: ''}"
		/>
	</button>
	{#if expanded && (toolArgs || result)}
		<div class="mt-1.5 ml-5 space-y-1.5">
			{#if toolArgs}
				<div>
					<p
						class="text-[10px] font-medium text-muted-foreground/40 uppercase tracking-wider mb-0.5"
					>
						Args
					</p>
					<pre
						class="text-xs text-muted-foreground/60 bg-muted-foreground/5 rounded p-2 overflow-x-auto whitespace-pre-wrap font-mono leading-relaxed">{formattedArgs ??
							toolArgs}</pre>
				</div>
			{/if}
			{#if result}
				<div>
					<p
						class="text-[10px] font-medium text-muted-foreground/40 uppercase tracking-wider mb-0.5"
					>
						Result
					</p>
					<pre
						class="text-xs {isError
							? 'text-red-500/70'
							: 'text-muted-foreground/60'} bg-muted-foreground/5 rounded p-2 overflow-x-auto whitespace-pre-wrap font-mono leading-relaxed">{formattedResult ??
							result}</pre>
				</div>
			{/if}
		</div>
	{/if}
</div>
