<script lang="ts">
	import { onMount } from "svelte";

	import AgentSidebar from "$lib/components/agent/agent-sidebar.svelte";
	import NewAgentDialog from "$lib/components/dialogs/new-agent-dialog.svelte";
	import McpApprovalDialog from "$lib/components/dialogs/mcp-approval-dialog.svelte";
	import Toolbar from "$lib/components/main/toolbar.svelte";
	import ChatPanel from "$lib/components/main/chat-panel.svelte";
	import GraphPanel from "$lib/components/main/graph-panel.svelte";
	import PinnedSidebar from "$lib/components/main/pinned-sidebar.svelte";

	import { state as app } from "$lib/state.svelte.js";

	let showSidebar = $state(true);
	let showGraph = $state(false);
	let showPins = $state(true);
	let panelWidth = $state(320);

	const STORAGE_KEY = "workflow-ui:layout";

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

	function handleResize(deltaX: number) {
		panelWidth = Math.max(180, panelWidth + deltaX);
	}

	function toggleSidebar() {
		showSidebar = !showSidebar;
	}

	function toggleGraph() {
		showGraph = !showGraph;
		if (showGraph) showPins = false;
	}

	function togglePins() {
		showPins = !showPins;
		if (showPins) showGraph = false;
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

	<div class="flex flex-col flex-1 min-w-0 min-h-0 overflow-hidden rounded-lg bg-background">
		<Toolbar
			{showSidebar}
			{showGraph}
			{showPins}
			onToggleSidebar={toggleSidebar}
			onToggleGraph={toggleGraph}
			onTogglePins={togglePins}
		/>

		<div class="flex flex-1 min-h-0">
			<ChatPanel
				empty={app.messages.length === 0}
				chatItems={app.chatItems}
				selected={app.selected}
				agents={app.agents}
				error={app.error}
				bind:input={app.input}
				pinnedMessages={app.pinnedMessages}
				pendingAction={app.pendingAction}
				running={app.running}
				onPin={(item) => app.togglePinMessage(item)}
				onDismissError={() => app.dismissError()}
				onSubmit={() => app.submit()}
				onStop={() => app.stop()}
			/>

			{#if showGraph}
				<GraphPanel
					agents={app.agents}
					statuses={app.agentStatuses}
					selected={app.selected}
					{panelWidth}
					onSelect={(id) => app.selectAgent(id)}
					onResize={handleResize}
				/>
			{/if}

			{#if showPins}
				<PinnedSidebar
					messages={app.pinnedMessages}
					{panelWidth}
					showResizeHandle={!showGraph}
					onUnpin={(pinId) => app.unpinMessage(pinId)}
					onResize={handleResize}
				/>
			{/if}
		</div>
	</div>
</div>
