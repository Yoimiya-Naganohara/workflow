<script lang="ts">
	import { onMount } from "svelte";
	import { goto } from "$app/navigation";
	import { Button } from "$lib/components/ui/button";
	import { Card } from "$lib/components/ui/card";
	import {
		MessageSquare,
		GitBranch,
		Settings,
		PanelLeftClose,
		PanelLeftOpen,
		Pin,
	} from "@lucide/svelte";
	import { formatRole } from "$lib/utils.js";

	import AgentSidebar from "$lib/components/agent/agent-sidebar.svelte";
	import AgentGraph from "$lib/components/agent/agent-graph.svelte";
	import PinnedPanel from "$lib/components/agent/pinned-panel.svelte";
	import ExecutionTimeline from "$lib/components/chat/execution-timeline.svelte";
	import ChatInput from "$lib/components/chat/chat-input.svelte";
	import NewAgentDialog from "$lib/components/dialogs/new-agent-dialog.svelte";
	import McpApprovalDialog from "$lib/components/dialogs/mcp-approval-dialog.svelte";

	import { state as app } from "$lib/state.svelte.js";

	let showSidebar = $state(true);
	let showGraph = $state(false);
	let showPins = $state(true);
	const STORAGE_KEY = "workflow-ui:layout";

	let rafId: number | null = null;
	function startResize(e: MouseEvent) {
		e.preventDefault();
		const startX = e.clientX;
		const startWidth = panelWidth;
		function onMove(ev: MouseEvent) {
			if (rafId != null) cancelAnimationFrame(rafId);
			rafId = requestAnimationFrame(() => {
				rafId = null;
				panelWidth = startWidth - (ev.clientX - startX);
			});
		}
		function onUp() {
			if (rafId != null) {
				cancelAnimationFrame(rafId);
				rafId = null;
			}
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

	let panelWidth = $state(320);

	function calcDefaultWidth() {
		if (typeof window === "undefined") return 320;
		const available = window.innerWidth - 240;
		return Math.floor(available / 2);
	}

	function saveLayout() {
		try {
			localStorage.setItem(
				STORAGE_KEY,
				JSON.stringify({ panelWidth, showGraph, showPins, showSidebar }),
			);
		} catch {
			/* ignore */
		}
	}

	$effect(() => {
		if (typeof window === "undefined") return;
		saveLayout();
	});

	onMount(() => {
		try {
			const saved = localStorage.getItem(STORAGE_KEY);
			if (saved) {
				const {
					panelWidth: w,
					showGraph: sg,
					showPins: sp,
					showSidebar: sb,
				} = JSON.parse(saved);
				if (typeof w === "number" && w >= 180) panelWidth = w;
				if (typeof sg === "boolean") showGraph = sg;
				if (typeof sp === "boolean") showPins = sp;
				if (typeof sb === "boolean") showSidebar = sb;
			} else {
				panelWidth = calcDefaultWidth();
			}
		} catch {
			panelWidth = calcDefaultWidth();
		}
		// Responsive auto-collapse
		const mq = window.matchMedia("(max-width: 1024px)");
		if (mq.matches) showSidebar = false;
		mq.addEventListener("change", (e) => {
			if (e.matches) showSidebar = false;
		});
		app.init();
		return () => app.destroy();
	});
</script>

<NewAgentDialog
	open={app.dialog === "new-agent"}
	roles={app.roles}
	onCreate={(role) => app.createAgent(role)}
	onOpenChange={(o) => {
		if (!o) app.closeDialog();
	}}
/>

<McpApprovalDialog
	open={app.dialog === "mcp-approval"}
	server={app.pendingMcpApproval?.server ?? ""}
	tool={app.pendingMcpApproval?.tool ?? ""}
	arguments={app.pendingMcpApproval?.arguments ?? {}}
	onApprove={() => app.approveMcpTool()}
	onDeny={() => app.denyMcpTool()}
	onOpenChange={(o) => {
		if (!o) {
			app.pendingMcpApproval = null;
			app.closeDialog();
		}
	}}
/>

<div class="fixed inset-0 top-8 flex flex-row p-1 gap-1 overflow-hidden">
	<div class="relative flex" class:hidden={!showSidebar}>
		<AgentSidebar
			agents={app.agents}
			selected={app.selected}
			statuses={app.agentStatuses}
			roles={app.roles}
			onSelect={(id) => app.selectAgent(id)}
			onCreateClick={() => app.openDialog("new-agent")}
			onRemoveAgent={(id) => app.removeAgent(id)}
			mcpConfigs={app.mcpConfigs}
			mcpConnections={app.mcpConnections}
			mcpExpanded={app.mcpExpanded}
			onToggleMcp={() => app.toggleMcp()}
			onAddMcpServer={(config) => app.addMcpServer(config)}
			onRemoveMcpServer={(name) => app.removeMcpServer(name)}
			projectPath={app.projectPath}
			projectName={app.projectName}
			projectExpanded={app.projectExpanded}
			onToggleProject={() => app.toggleProject()}
			onChangeProject={(path) => app.reconfigureProject(path)}
			onClearProject={() => app.clearProject()}
		/>
	</div>

	<div
		class="flex flex-col flex-1 min-w-0 min-h-0 overflow-hidden rounded-lg bg-background"
	>
		<div
			class="flex items-center gap-0.5 px-2 py-1 border-b border-border bg-card shrink-0"
		>
			<Button
				variant="ghost"
				size="icon-xs"
				class="text-muted-foreground/50 hover:text-muted-foreground shrink-0"
				onclick={() => (showSidebar = !showSidebar)}
				title={showSidebar ? "Hide sidebar" : "Show sidebar"}
				aria-label={showSidebar ? "Hide sidebar" : "Show sidebar"}
			>
				{#if showSidebar}
					<PanelLeftClose class="size-3.5" />
				{:else}
					<PanelLeftOpen class="size-3.5" />
				{/if}
			</Button>
			<div class="flex-1"></div>
			<Button
				variant="ghost"
				size="icon-xs"
				class={showGraph
					? "bg-accent text-accent-foreground"
					: "text-muted-foreground/50"}
				onclick={() => {
					showGraph = !showGraph;
					if (showGraph) showPins = false;
				}}
				title={showGraph ? "Hide graph" : "Show graph"}
			>
				<GitBranch class="size-3.5" />
			</Button>
			<Button
				variant="ghost"
				size="icon-xs"
				class={showPins
					? "bg-accent text-accent-foreground"
					: "text-muted-foreground/50"}
				onclick={() => {
					showPins = !showPins;
					if (showPins) showGraph = false;
				}}
				title={showPins ? "Hide pinned messages" : "Show pinned messages"}
			>
				<Pin class="size-3.5" />
			</Button>
			<Button
				variant="ghost"
				size="icon-xs"
				onclick={() => goto("/settings")}
				title="Settings"
				aria-label="Settings"
			>
				<Settings class="size-3.5" />
			</Button>
		</div>

		<div class="flex flex-1 min-h-0">
			<div
				class="flex flex-col flex-1 min-w-0 min-h-0"
				class:justify-center={app.messages.length === 0}
			>
				{#if app.messages.length > 0}
					<ExecutionTimeline
						items={app.chatItems}
						empty={false}
						agentRole={app.agents.find(a => a.id === app.selected)?.role}
						pinnedMessages={app.pinnedMessages}
						onPin={(item) => app.togglePinMessage(item)}
					/>
				{/if}

				{#if app.error}
					<div class="animate-in shrink-0 mx-3 mb-2">
						<div
							class="flex items-start gap-2.5 rounded-lg bg-destructive/8 border border-destructive/20 px-3 py-2"
						>
							<svg
								xmlns="http://www.w3.org/2000/svg"
								viewBox="0 0 24 24"
								fill="none"
								stroke="currentColor"
								stroke-width="2"
								stroke-linecap="round"
								stroke-linejoin="round"
								class="size-3.5 shrink-0 mt-0.5 text-destructive"
								><path
									d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3"
								/><path d="M12 9v4" /><path
									d="M12 17h.01"
								/></svg
							>
							<p
								class="flex-1 text-xs text-destructive leading-relaxed"
							>
								{app.error}
							</p>
							<button
								onclick={() => app.dismissError()}
								class="shrink-0 mt-0.5 text-destructive/50 hover:text-destructive transition-colors"
								aria-label="Dismiss error"
							>
								<svg
									xmlns="http://www.w3.org/2000/svg"
									viewBox="0 0 24 24"
									fill="none"
									stroke="currentColor"
									stroke-width="2"
									stroke-linecap="round"
									stroke-linejoin="round"
									class="size-3"
									><path d="M18 6 6 18" /><path
										d="m6 6 12 12"
									/></svg
								>
							</button>
						</div>
					</div>
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

			{#if showGraph}
				<div
					class="relative shrink-0 bg-background flex flex-col min-h-0"
					style="width: {panelWidth}px"
				>
					<div
						role="presentation"
						class="absolute inset-y-0 -left-[3px] w-[7px] z-10 cursor-col-resize group"
						onmousedown={startResize}
					>
						<div
							class="absolute inset-y-0 left-1/2 -translate-x-1/2 w-px bg-border group-hover:bg-accent-foreground/20 group-active:bg-accent-foreground/30 transition-colors"
						></div>
						<div
							class="absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 flex flex-col gap-[3px] opacity-0 group-hover:opacity-100 transition-opacity"
						>
							<div
								class="size-[3px] rounded-full bg-accent-foreground/30"
							></div>
							<div
								class="size-[3px] rounded-full bg-accent-foreground/30"
							></div>
							<div
								class="size-[3px] rounded-full bg-accent-foreground/30"
							></div>
						</div>
					</div>
					<AgentGraph
						agents={app.agents}
						statuses={app.agentStatuses}
						selected={app.selected}
						onSelect={(id) => app.selectAgent(id)}
					/>
					{#if app.selected != null}
						{@const agent = app.agents.find(
							(a) => a.id === app.selected,
						)}
						{@const status =
							app.agentStatuses.get(app.selected) ?? "idle"}
						{@const statusColor =
							status === "thinking" || status === "running-tool"
								? "#f59e0b"
								: status === "responding"
									? "#22c55e"
									: status === "error"
										? "#ef4444"
										: "#6b7280"}
						<div class="shrink-0 border-t border-border p-3 space-y-2">
							<div class="flex items-center justify-between">
								<div class="flex items-center gap-2">
									<div
										class="size-2 rounded-full"
										style="background: {statusColor}"
									></div>
									<span class="text-xs font-medium"
										>#{agent?.id}
										{agent ? formatRole(agent.role) : ""}</span
									>
								</div>
								<span
									class="text-[10px] text-muted-foreground/50 capitalize"
									>{status.replace("-", " ")}</span
								>
							</div>
							{#if agent?.current_task}
								<div
									class="text-xs text-muted-foreground/70 bg-muted/50 rounded px-2 py-1.5 truncate"
									title={agent.current_task}
								>
									{agent.current_task}
								</div>
							{/if}
						</div>
					{/if}
				</div>
			{/if}

			{#if showPins}
				<div
					class="relative shrink-0 bg-background flex flex-col min-h-0"
					style="width: {panelWidth}px"
				>
					{#if !showGraph}
						<div
							role="presentation"
							class="absolute inset-y-0 -left-[3px] w-[7px] z-10 cursor-col-resize group"
							onmousedown={startResize}
						>
							<div
								class="absolute inset-y-0 left-1/2 -translate-x-1/2 w-px bg-border group-hover:bg-accent-foreground/20 group-active:bg-accent-foreground/30 transition-colors"
							></div>
							<div
								class="absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 flex flex-col gap-[3px] opacity-0 group-hover:opacity-100 transition-opacity"
							>
								<div
									class="size-[3px] rounded-full bg-accent-foreground/30"
								></div>
								<div
									class="size-[3px] rounded-full bg-accent-foreground/30"
								></div>
								<div
									class="size-[3px] rounded-full bg-accent-foreground/30"
								></div>
							</div>
						</div>
					{/if}
					<PinnedPanel
						messages={app.pinnedMessages}
						onUnpin={(pinId) => app.unpinMessage(pinId)}
						empty={app.pinnedMessages.length === 0}
					/>
				</div>
			{/if}
		</div>
	</div>
</div>
