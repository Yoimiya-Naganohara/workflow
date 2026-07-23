<script lang="ts">
	import { onMount } from "svelte";
	import { Button } from "$lib/components/ui/button";
	import { Card } from "$lib/components/ui/card";
	import { MessageSquare, GitBranch, Settings, Eye, EyeOff } from "@lucide/svelte";
	import { formatRole } from "$lib/utils.js";

	import AgentSidebar from "$lib/components/agent/agent-sidebar.svelte";
	import AgentGraph from "$lib/components/agent/agent-graph.svelte";
	import ExecutionTimeline from "$lib/components/chat/execution-timeline.svelte";
	import ChatInput from "$lib/components/chat/chat-input.svelte";
	import NewAgentDialog from "$lib/components/dialogs/new-agent-dialog.svelte";
	import SettingsDialog from "$lib/components/dialogs/settings-dialog.svelte";
	import RolesDialog from "$lib/components/dialogs/roles-dialog.svelte";

	import { state as app } from "$lib/state.svelte.js";

	let showGraph = $state(true);

	let rafId: number | null = null;
	function startResize(e: MouseEvent) {
		e.preventDefault();
		const startX = e.clientX;
		const startWidth = graphWidth;
		function onMove(ev: MouseEvent) {
			if (rafId != null) cancelAnimationFrame(rafId);
			rafId = requestAnimationFrame(() => {
				rafId = null;
				graphWidth = startWidth - (ev.clientX - startX);
			});
		}
		function onUp() {
			if (rafId != null) { cancelAnimationFrame(rafId); rafId = null; }
			document.removeEventListener("mousemove", onMove);
			document.removeEventListener("mouseup", onUp);
			document.body.style.cursor = "";
			document.body.style.userSelect = "";
		}
		document.addEventListener("mousemove", onMove);
		document.addEventListener("mouseup", onUp);
		document.body.style.cursor = "col-resize";
		document.body.style.userSelect = "none";
	}

	let graphWidth = $state(320);

	function calcDefaultWidth() {
		if (typeof window === "undefined") return 320;
		const available = window.innerWidth - 240;
		return Math.floor(available / 2);
	}

	onMount(() => {
		graphWidth = calcDefaultWidth();
		app.init();
		return () => app.destroy();
	});
</script>

<NewAgentDialog
	open={app.dialog === "new-agent"}
	roles={app.roles}
	onCreate={(role) => app.createAgent(role)}
	onOpenChange={(o) => { if (!o) app.closeDialog(); }}
/>

<SettingsDialog
	open={app.dialog === "settings"}
	providers={app.providers}
	selectedProvider={app.selectedProvider}
	selectedModel={app.selectedModel}
	apiKey={app.settingsApiKey}
	configured={app.configured}
	refreshing={app.pendingAction?.type === "refresh-providers"}
	onOpenChange={(o) => { if (!o) app.closeDialog(); }}
	onConfigure={(pid, key, model) => app.configureRuntime(pid, key, model)}
	onRefreshProviders={() => app.refreshProviders()}
/>

<RolesDialog
	open={app.dialog === "roles"}
	roles={app.roles}
	onAddRole={(name, def) => app.addRole(name, def)}
	onOpenChange={(o) => { if (!o) app.closeDialog(); }}
/>

<div class="fixed inset-0 top-8 flex flex-row p-1 gap-1 overflow-hidden">
	<AgentSidebar
		agents={app.agents}
		selected={app.selected}
		statuses={app.agentStatuses}
		roles={app.roles}
		rolesExpanded={app.rolesExpanded}
		onSelect={(id) => app.selectAgent(id)}
		onCreateClick={() => app.openDialog("new-agent")}
		onRemoveAgent={(id) => app.removeAgent(id)}
		onRolesClick={() => app.openDialog("roles")}
		onToggleRoles={() => app.toggleRoles()}
	/>

	<div class="flex flex-col flex-1 min-w-0 min-h-0 overflow-hidden rounded-lg bg-background">
		<div class="flex items-center gap-0.5 px-2 py-1 border-b border-border bg-card shrink-0">
			<div class="flex items-center gap-1.5 min-w-0">
				<MessageSquare class="size-3.5 text-muted-foreground shrink-0" />
				<span class="text-xs font-medium text-muted-foreground">Chat</span>
			</div>
			<div class="flex items-center gap-1.5 ml-4 pl-4 border-l border-border">
				<GitBranch class="size-3.5 text-muted-foreground shrink-0" />
				<span class="text-xs font-medium text-muted-foreground">Graph</span>
			</div>
			{#if app.mcpServers.length > 0}
				<div class="flex items-center gap-1 ml-4 pl-4 border-l border-border">
					<span class="size-2 rounded-full bg-cyan-500"></span>
					<span class="text-xs text-muted-foreground/70 tabular-nums">
						{app.mcpServers.length} MCP
					</span>
				</div>
			{/if}
			<div class="flex-1"></div>
			<Button
				variant="ghost"
				size="icon-xs"
				class={showGraph ? "bg-accent text-accent-foreground" : "text-muted-foreground/50"}
				onclick={() => (showGraph = !showGraph)}
				title={showGraph ? "Hide graph" : "Show graph"}
			>
				{#if showGraph}
					<EyeOff class="size-3.5" />
				{:else}
					<Eye class="size-3.5" />
				{/if}
			</Button>
			<Button variant="ghost" size="icon-xs" onclick={() => app.openDialog("settings")}>
				<Settings class="size-3.5" />
			</Button>
		</div>

		<div class="flex flex-1 min-h-0">
			<div class="flex flex-col flex-1 min-w-0 min-h-0" class:justify-center={app.messages.length === 0}>
				{#if app.messages.length > 0}
					<ExecutionTimeline
						items={app.chatItems}
						empty={false}
						agentId={app.selected}
						agentRole={app.agents.find(a => a.id === app.selected)?.role}
					/>
				{/if}

				{#if app.error}
					<Card class="shrink-0 mx-3 mb-2 px-3 py-2 bg-destructive/5 border-destructive/20 border-dashed">
						<p class="text-xs text-destructive">{app.error}</p>
					</Card>
				{/if}

				<div class="shrink-0 px-3 pb-3">
					<ChatInput
						bind:value={app.input}
						disabled={app.selected == null}
						pendingAction={app.pendingAction}
						showStop={app.running}
						onSubmit={() => app.submit()}
						onStop={() => app.stop()}
					/>
				</div>
			</div>

			<div
				class="relative shrink-0 bg-background flex flex-col min-h-0"
				class:hidden={!showGraph}
				style="width: {graphWidth}px"
			>
				<div
					role="presentation"
					class="absolute inset-y-0 -left-[3px] w-[7px] z-10 cursor-col-resize group"
					onmousedown={startResize}
				>
					<div class="absolute inset-y-0 left-1/2 -translate-x-1/2 w-px bg-border group-hover:bg-accent-foreground/20 group-active:bg-accent-foreground/30 transition-colors"></div>
					<div class="absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 flex flex-col gap-[3px] opacity-0 group-hover:opacity-100 transition-opacity">
						<div class="size-[3px] rounded-full bg-accent-foreground/30"></div>
						<div class="size-[3px] rounded-full bg-accent-foreground/30"></div>
						<div class="size-[3px] rounded-full bg-accent-foreground/30"></div>
					</div>
				</div>
				<AgentGraph
					agents={app.agents}
					statuses={app.agentStatuses}
					selected={app.selected}
					onSelect={(id) => app.selectAgent(id)}
				/>
				{#if app.selected != null}
					{@const agent = app.agents.find(a => a.id === app.selected)}
					{@const status = app.agentStatuses.get(app.selected) ?? "idle"}
					{@const statusColor = status === "thinking" || status === "running-tool" ? "#f59e0b" : status === "responding" ? "#22c55e" : status === "error" ? "#ef4444" : "#6b7280"}
					<div class="shrink-0 border-t border-border p-3 space-y-2">
						<div class="flex items-center justify-between">
							<div class="flex items-center gap-2">
								<div class="size-2 rounded-full" style="background: {statusColor}"></div>
								<span class="text-xs font-medium">#{agent?.id} {agent ? formatRole(agent.role) : ""}</span>
							</div>
							<span class="text-[10px] text-muted-foreground/50 capitalize">{status.replace("-", " ")}</span>
						</div>
						{#if agent?.current_task}
							<div class="text-xs text-muted-foreground/70 bg-muted/50 rounded px-2 py-1.5 truncate" title={agent.current_task}>
								{agent.current_task}
							</div>
						{/if}
					</div>
				{/if}
			</div>
		</div>
	</div>
</div>
