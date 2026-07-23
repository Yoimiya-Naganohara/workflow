<script lang="ts">
	import { Button } from "$lib/components/ui/button";
	import { Card } from "$lib/components/ui/card";
	import { Shield, ShieldOff } from "@lucide/svelte";

	let {
		open,
		server,
		tool,
		arguments: args,
		onApprove,
		onDeny,
		onOpenChange,
	}: {
		open: boolean;
		server: string;
		tool: string;
		arguments: Record<string, unknown>;
		onApprove: () => void;
		onDeny: () => void;
		onOpenChange: (open: boolean) => void;
	} = $props();

	function handleKeydown(e: KeyboardEvent) {
		if (!open) return;
		if (e.key === "Escape") {
			onOpenChange(false);
		}
	}

	$effect(() => {
		if (open) {
			document.addEventListener("keydown", handleKeydown);
			return () => document.removeEventListener("keydown", handleKeydown);
		}
	});
</script>

{#if open}
	<div
		class="fixed inset-0 z-50 flex items-center justify-center bg-black/50"
		role="none"
		onclick={() => onOpenChange(false)}
		onkeydown={(e) => { if (e.key === 'Escape') onOpenChange(false); }}
	>
		<!-- svelte-ignore a11y_interactive_supports_focus -->
		<div
			class="w-full max-w-md mx-4 rounded-xl border border-border bg-card shadow-2xl"
			role="dialog"
			aria-modal="true"
			aria-label="MCP tool approval"
			tabindex="-1"
			onclick={(e) => e.stopPropagation()}
			onkeydown={(e) => { if (e.key === 'Escape') onOpenChange(false); }}
		>
			<div class="flex items-center gap-2 px-5 py-3 border-b border-border">
				<Shield class="size-4 text-amber-500" />
				<span class="text-sm font-semibold">MCP Tool Requires Approval</span>
			</div>

			<div class="p-5 space-y-3">
				<div class="flex items-center gap-2 text-sm">
					<span class="text-muted-foreground w-16 shrink-0">Server:</span>
					<code class="px-1.5 py-0.5 rounded bg-muted text-xs font-mono">{server}</code>
				</div>
				<div class="flex items-center gap-2 text-sm">
					<span class="text-muted-foreground w-16 shrink-0">Tool:</span>
					<code class="px-1.5 py-0.5 rounded bg-muted text-xs font-mono">{tool}</code>
				</div>
				<div class="text-sm">
					<span class="text-muted-foreground block mb-1">Arguments:</span>
					<Card class="p-2">
						<pre class="text-xs font-mono whitespace-pre-wrap break-all max-h-48 overflow-y-auto">{JSON.stringify(args, null, 2)}</pre>
					</Card>
				</div>
			</div>

			<div class="flex items-center justify-end gap-2 px-5 py-3 border-t border-border">
				<Button variant="outline" size="sm" onclick={onDeny}>
					<ShieldOff class="size-3.5 mr-1" />
					Deny
				</Button>
				<Button variant="default" size="sm" onclick={onApprove}>
					<Shield class="size-3.5 mr-1" />
					Approve
				</Button>
			</div>
		</div>
	</div>
{/if}
