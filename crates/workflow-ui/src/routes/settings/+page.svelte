<script lang="ts">
	import { goto } from "$app/navigation";
	import { invoke } from "@tauri-apps/api/core";
	import { ArrowLeft, Database, Plug, User } from "@lucide/svelte";
	import { onMount } from "svelte";
	import { Button } from "$lib/components/ui/button";
	import { cn } from "$lib/utils";
	import type { ProviderEntry, RoleInfo } from "$lib/types";
	import { state as app } from "$lib/state.svelte.js";

	import ProviderTab from "$lib/components/settings/provider-tab.svelte";
	import RolesTab from "$lib/components/settings/roles-tab.svelte";
	import McpTab from "$lib/components/settings/mcp-tab.svelte";

	// ── Tabs ────────────────────────────────────────────
	const TABS = [
		{ id: "provider", label: "Provider", icon: Database },
		{ id: "mcp", label: "MCP", icon: Plug },
		{ id: "roles", label: "Roles", icon: User },
	];
	let activeTab = $state("provider");

	// ── Provider state ──────────────────────────────────
	let providers = $state<ProviderEntry[]>([]);
	let refreshing = $state(false);

	async function loadProviders() {
		refreshing = true;
		try {
			providers = await invoke<ProviderEntry[]>("list_providers");
			if (providers.length === 0) {
				providers = await invoke<ProviderEntry[]>("fetch_providers");
			}
		} catch {
			providers = [];
		} finally {
			refreshing = false;
		}
	}

	async function handleRefresh() {
		refreshing = true;
		try {
			providers = await invoke<ProviderEntry[]>("fetch_providers");
		} catch {
			providers = [];
		} finally {
			refreshing = false;
		}
	}

	// ── Roles state ─────────────────────────────────────
	let roles = $state<RoleInfo[]>([]);

	async function loadRoles() {
		try {
			roles = await invoke<RoleInfo[]>("load_roles");
			app.roles = roles;
		} catch {
			roles = [];
		}
	}

	async function addRole(name: string, def: string) {
		try {
			roles = await invoke<RoleInfo[]>("add_role", { name, definition: def });
			app.roles = roles;
		} catch (e) {
			console.error("add role:", e);
		}
	}

	async function deleteRole(name: string) {
		try {
			roles = await invoke<RoleInfo[]>("remove_role", { name });
			app.roles = roles;
		} catch (e) {
			console.error("delete role:", e);
		}
	}

	// ── Init ────────────────────────────────────────────
	onMount(async () => {
		await Promise.all([loadProviders(), loadRoles()]);
	});

	function handleBack() {
		goto("/");
	}
</script>

<div class="h-full w-full flex flex-col">
	<!-- Header -->
	<div class="flex items-center gap-3 px-4 py-3 border-b border-border shrink-0">
		<Button variant="ghost" size="icon-xs" onclick={handleBack} title="Back to chat">
			<ArrowLeft class="size-4" />
		</Button>
		<h1 class="text-sm font-semibold">Settings</h1>
	</div>

	<div class="flex flex-1 min-h-0">
		<!-- Tab sidebar -->
		<nav class="w-44 shrink-0 border-r border-border bg-muted/20 p-2 space-y-1 overflow-y-auto">
			{#each TABS as { id, label, icon: Icon } (id)}
				<button
					class={cn(
						"w-full flex items-center gap-2.5 px-3 py-2 rounded-md text-xs font-medium transition-colors text-left",
						activeTab === id
							? "bg-accent text-accent-foreground shadow-sm"
							: "text-muted-foreground hover:text-foreground hover:bg-muted/50",
					)}
					onclick={() => { activeTab = id; }}
				>
					<Icon class="size-3.5 shrink-0" />
					{label}
				</button>
			{/each}
		</nav>

		<!-- Content -->
		<div class="flex-1 min-h-0 overflow-y-auto m-6">
			{#if activeTab === "provider"}
				<ProviderTab
					bind:localProvider={app.selectedProvider}
					bind:localModel={app.selectedModel}
					bind:localApiKey={app.settingsApiKey}
					{providers}
					{refreshing}
					configured={app.configured}
					onRefresh={handleRefresh}
				/>
			{:else if activeTab === "mcp"}
				<McpTab
					configs={app.mcpConfigs}
					connections={app.mcpConnections}
					onAdd={(config) => app.addMcpServer(config)}
					onRemove={(name) => app.removeMcpServer(name)}
				/>
			{:else}
				<RolesTab
					{roles}
					onAddRole={addRole}
					onDeleteRole={deleteRole}
				/>
			{/if}
		</div>
	</div>
</div>
