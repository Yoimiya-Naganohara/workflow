<script lang="ts">
	import { onMount } from "svelte";
	import { cn } from "$lib/utils.js";
	import { Button } from "$lib/components/ui/button";
	import { Card } from "$lib/components/ui/card";
	import { Blocks, MessageSquare, Settings } from "@lucide/svelte";

	import AgentSidebar from "$lib/components/agent/agent-sidebar.svelte";
	import ExecutionTimeline from "$lib/components/chat/execution-timeline.svelte";
	import ChatInput from "$lib/components/chat/chat-input.svelte";
	import NewAgentDialog from "$lib/components/dialogs/new-agent-dialog.svelte";
	import SettingsDialog from "$lib/components/dialogs/settings-dialog.svelte";
	import RolesDialog from "$lib/components/dialogs/roles-dialog.svelte";

	import { state as app } from "$lib/state.svelte.js";

	let orchestrateInput = $state("");

	onMount(() => {
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
	onLoadProviders={() => app.loadProviders()}
	onRefreshProviders={() => app.refreshProviders()}
/>

<RolesDialog
	open={app.dialog === "roles"}
	roles={app.roles}
	onAddRole={(name, def) => app.addRole(name, def)}
	onOpenChange={(o) => { if (!o) app.closeDialog(); }}
/>

<div class="fixed inset-0 top-11 flex flex-row bg-background overflow-hidden">
	<AgentSidebar
		agents={app.agents}
		selected={app.selected}
		statuses={app.agentStatuses}
		roles={app.roles}
		rolesExpanded={app.rolesExpanded}
		onSelect={(id) => app.selectAgent(id)}
		onRemove={(id) => app.removeAgent(id)}
		onCreateClick={() => app.openDialog("new-agent")}
		onRolesClick={() => app.openDialog("roles")}
		onToggleRoles={() => app.toggleRoles()}
	/>

	<div class="flex flex-col flex-1 min-w-0 min-h-0 overflow-hidden">
		<div class="flex items-center gap-0.5 px-2 py-1 border-b border-border bg-card shrink-0">
			<Button
				variant="ghost"
				size="sm"
				class={cn("text-xs", app.currentTab === "chat" && "bg-accent text-accent-foreground")}
				onclick={() => app.setTab("chat")}
			>
				<MessageSquare class="size-3.5" />
				Chat
			</Button>
			<Button
				variant="ghost"
				size="sm"
				class={cn("text-xs", app.currentTab === "orchestrate" && "bg-accent text-accent-foreground")}
				onclick={() => app.setTab("orchestrate")}
			>
				<Blocks class="size-3.5" />
				Orchestrate
			</Button>
			<div class="flex-1"></div>
			<Button variant="ghost" size="icon-xs" onclick={() => app.openDialog("settings")}>
				<Settings class="size-3.5" />
			</Button>
		</div>

		{#if app.currentTab === "chat"}
			<div class="flex flex-col flex-1 min-h-0">
				<ExecutionTimeline
					items={app.chatItems}
					empty={app.messages.length === 0}
					agentId={app.selected}
					agentRole={app.agents.find(a => a.id === app.selected)?.role}
				/>

				{#if app.error}
					<Card class="shrink-0 mx-3 mb-2 px-3 py-2 bg-destructive/5 border-destructive/20 border-dashed">
						<p class="text-xs text-destructive">{app.error}</p>
					</Card>
				{/if}

				<div class="shrink-0 bg-card border-t border-border p-3">
					<ChatInput
						bind:value={app.input}
						disabled={app.selected == null}
						pendingAction={app.pendingAction}
						onSubmit={() => app.submit()}
					/>
				</div>
			</div>
		{:else}
			<div class="flex-1 flex items-center justify-center p-8">
				<Card class="flex flex-col items-center gap-3 text-center py-12 px-8 max-w-sm border-dashed">
					<div class="size-12 rounded-full bg-muted flex items-center justify-center">
						<Blocks class="size-6 text-muted-foreground/40" />
					</div>
					<div>
						<p class="text-sm font-medium text-muted-foreground">Mission Orchestration</p>
						<p class="text-xs text-muted-foreground/60 mt-1">
							Describe a multi-agent workflow in natural language.
							Sub-agents will be created and tasks executed in dependency order.
						</p>
					</div>
					<div class="w-full mt-2 space-y-2">
						<textarea
							bind:value={orchestrateInput}
							placeholder={`Example:\nBuild a web app:\n1. Research requirements\n2. Design architecture\n3. Implement backend\n4. Build frontend`}
							class="min-h-28 resize-none text-sm w-full rounded-lg border border-input bg-transparent px-2.5 py-2 transition-colors focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-3 outline-none placeholder:text-muted-foreground disabled:opacity-50"
							disabled={app.selected == null || app.pendingAction?.type === "send"}
						></textarea>
						<Button
							class="w-full"
							disabled={!orchestrateInput.trim() || app.selected == null || app.pendingAction?.type === "send"}
							onclick={() => app.startOrchestration(orchestrateInput.trim())}
						>
							<Blocks class="size-3.5" /> Orchestrate
						</Button>
						{#if app.selected == null}
							<p class="text-[10px] text-muted-foreground/50">Select an agent from the sidebar first.</p>
						{/if}
					</div>
				</Card>
			</div>
		{/if}
	</div>
</div>
