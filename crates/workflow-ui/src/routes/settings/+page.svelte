<script lang="ts">
	import { goto } from "$app/navigation";
	import { invoke } from "@tauri-apps/api/core";
	import ArrowLeft from "@lucide/svelte/icons/arrow-left";
	import { onMount } from "svelte";
	import { Button } from "$lib/components/ui/button";
	import { cn } from "$lib/utils";
	import type { ProviderEntry, RoleInfo } from "$lib/types";
	import { state as app } from "$lib/state.svelte.js";

	import ProviderTab from "$lib/components/settings/provider-tab.svelte";
	import RolesTab from "$lib/components/settings/roles-tab.svelte";

	// ── Tabs ────────────────────────────────────────────
	const TABS = [
		{ id: "provider", label: "Provider" },
		{ id: "roles", label: "Roles" },
	] as const;
	let activeTab = $state("provider");

	// ── Provider state ──────────────────────────────────
	let providers = $state<ProviderEntry[]>([]);
	let refreshing = $state(false);
	let saving = $state(false);
	let saveError = $state("");

	let localProvider = $state(app.selectedProvider);
	let localModel = $state(app.selectedModel);
	let localApiKey = $state(app.settingsApiKey);
	const needsApiKey = $derived(!!providers.find((p) => p.id === localProvider)?.api_url);

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
			saveError = `add role: ${e}`;
		}
	}

	async function deleteRole(name: string) {
		try {
			roles = await invoke<RoleInfo[]>("remove_role", { name });
			app.roles = roles;
		} catch (e) {
			saveError = `delete: ${e}`;
		}
	}

	// ── Init ────────────────────────────────────────────
	onMount(async () => {
		await Promise.all([loadProviders(), loadRoles()]);
	});

	// ── Actions ─────────────────────────────────────────
	async function handleSave() {
		saving = true;
		saveError = "";
		try {
			await app.configureRuntime(localProvider, localApiKey, localModel);
			goto("/");
		} catch (e) {
			saveError = `${e}`;
		} finally {
			saving = false;
		}
	}

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
			{#each TABS as tab}
				<button
					class={cn(
						"w-full flex items-center gap-2.5 px-3 py-2 rounded-md text-xs font-medium transition-colors text-left",
						activeTab === tab.id
							? "bg-accent text-accent-foreground shadow-sm"
							: "text-muted-foreground hover:text-foreground hover:bg-muted/50",
					)}
					onclick={() => { activeTab = tab.id; }}
				>
					{#if tab.id === "provider"}
						<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-3.5 shrink-0"><rect x="3" y="3" width="18" height="18" rx="2" ry="2" /><line x1="3" y1="9" x2="21" y2="9" /><line x1="9" y1="21" x2="9" y2="9" /></svg>
					{:else}
						<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-3.5 shrink-0"><path d="M12 2a4 4 0 1 0 0 8 4 4 0 0 0 0-8z" /><path d="M16 20v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2" /></svg>
					{/if}
					{tab.label}
				</button>
			{/each}
		</nav>

		<!-- Content -->
		<div class="flex-1 min-h-0 overflow-y-auto m-6">
			{#if activeTab === "provider"}
				<ProviderTab
					bind:localProvider
					bind:localModel
					bind:localApiKey
					{providers}
					{refreshing}
					configured={app.configured}
					onRefresh={handleRefresh}
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

	<!-- Bottom bar -->
	<div class="shrink-0 border-t border-border px-6 py-3 flex items-center justify-end gap-2">
		{#if saveError}
			<p class="text-xs text-red-500 flex-1">{saveError}</p>
		{/if}
		<Button variant="outline" onclick={handleBack} disabled={saving}>Cancel</Button>
		<Button
			disabled={saving || !localProvider || !localModel || (needsApiKey && !localApiKey)}
			onclick={handleSave}
		>
			{#if saving}
				<svg class="size-3.5 animate-spin mr-1" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 12a9 9 0 1 1-6.219-8.56" /></svg>
			{/if}
			Save
		</Button>
	</div>
</div>
