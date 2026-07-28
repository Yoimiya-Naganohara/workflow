<script lang="ts">
    import { goto } from "$app/navigation";
    import { invoke } from "@tauri-apps/api/core";
    import CheckIcon from "@lucide/svelte/icons/check";
    import ChevronsUpDownIcon from "@lucide/svelte/icons/chevrons-up-down";
    import ArrowLeft from "@lucide/svelte/icons/arrow-left";
    import { onMount, tick } from "svelte";
    import * as Command from "$lib/components/ui/command";
    import * as Popover from "$lib/components/ui/popover";
    import { Button } from "$lib/components/ui/button";
    import { Input } from "$lib/components/ui/input";
    import { Textarea } from "$lib/components/ui/textarea";
    import { Card } from "$lib/components/ui/card";
    import { cn } from "$lib/utils";
    import type { ProviderEntry, ProviderModel, RoleInfo } from "$lib/types";
    import { state as app } from "$lib/state.svelte.js";

    // ── Tabs ──────────────────────────────────────────────────────
    const TABS = [
        { id: "provider", label: "Provider" },
        { id: "roles", label: "Roles" },
    ] as const;
    let activeTab = $state("provider");

    // ── Provider state ───────────────────────────────────────────
    let providers = $state<ProviderEntry[]>([]);
    let refreshing = $state(false);
    let saving = $state(false);
    let saveError = $state("");
    let loading = $derived(providers.length === 0 && refreshing);

    let localProvider = $state(app.selectedProvider);
    let localModel = $state(app.selectedModel);
    let localApiKey = $state(app.settingsApiKey);

    const currentProvider = $derived(providers.find(p => p.id === localProvider));
    const availableModels = $derived(currentProvider?.models ?? []);
    const needsApiKey = $derived(!!currentProvider?.api_url);

    let providerOpen = $state(false);
    let modelOpen = $state(false);
    let providerTriggerRef = $state<HTMLButtonElement>(null!);
    let modelTriggerRef = $state<HTMLButtonElement>(null!);

    const selectedProviderName = $derived(providers.find(p => p.id === localProvider)?.name);
    const selectedModelName = $derived(currentProvider?.models.find((m: ProviderModel) => m.id === localModel)?.name);

    // ── Roles state ──────────────────────────────────────────────
    let roles = $state<RoleInfo[]>([]);
    let newRoleName = $state("");
    let newRoleDef = $state("");
    let editingId = $state<string | null>(null);
    let editingName = $state("");
    let editingDef = $state("");

    // ── Init ─────────────────────────────────────────────────────
    onMount(async () => {
        await Promise.all([loadProviders(), loadRoles()]);
    });

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

    function closeAndFocus(ref: HTMLButtonElement) {
        tick().then(() => ref.focus());
    }

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

    async function handleBack() {
        goto("/");
    }

    // ── Roles handlers ───────────────────────────────────────────
    function handleAddRole() {
        if (!newRoleName.trim() || !newRoleDef.trim()) return;
        addRole(newRoleName.trim(), newRoleDef.trim());
        newRoleName = "";
        newRoleDef = "";
    }

    function startEdit(r: RoleInfo) {
        editingId = r.id;
        editingName = r.name;
        editingDef = r.definition;
    }

    function cancelEdit() {
        editingId = null;
    }

    function saveEdit() {
        if (!editingId || !editingName.trim() || !editingDef.trim()) return;
        addRole(editingName.trim(), editingDef.trim());
        editingId = null;
    }
</script>

<div class="flex flex-col flex-1 min-h-0">
    <!-- Header -->
    <div class="flex items-center gap-3 px-4 py-3 border-b border-border shrink-0">
        <Button variant="ghost" size="icon-xs" onclick={handleBack} title="Back to chat">
            <ArrowLeft class="size-4" />
        </Button>
        <h1 class="text-sm font-semibold">Settings</h1>
    </div>

    <div class="flex flex-1 min-h-0">
        <!-- Tab sidebar -->
        <nav class="w-44 shrink-0 border-r border-border bg-muted/20 p-2 space-y-1">
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
                        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-3.5 shrink-0"><rect x="3" y="3" width="18" height="18" rx="2" ry="2"/><line x1="3" y1="9" x2="21" y2="9"/><line x1="9" y1="21" x2="9" y2="9"/></svg>
                    {:else}
                        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-3.5 shrink-0"><path d="M12 2a4 4 0 1 0 0 8 4 4 0 0 0 0-8z"/><path d="M16 20v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/></svg>
                    {/if}
                    {tab.label}
                </button>
            {/each}
        </nav>

        <!-- Content -->
        <div class="flex-1 overflow-y-auto p-6">
            <!-- ════════ Provider tab ════════ -->
            {#if activeTab === "provider"}
                <div class="max-w-xl mx-auto space-y-6">
                    <div>
                        <h2 class="text-base font-semibold">Provider</h2>
                        <p class="text-xs text-muted-foreground/60 mt-0.5">
                            Select an LLM provider and configure your API key.
                        </p>
                    </div>

                    <Card class="p-4 space-y-4">
                        <div class="flex items-center justify-between">
                            <span class="text-xs font-medium text-muted-foreground">Available Providers</span>
                            <Button variant="ghost" size="icon-xs" disabled={refreshing} onclick={handleRefresh}>
                                <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-3">
                                    <path d="M21 2v6h-6" /><path d="M3 12a9 9 0 0 1 15-6.7L21 8" />
                                    <path d="M3 22v-6h6" /><path d="M21 12a9 9 0 0 1-15 6.7L3 16" />
                                </svg>
                            </Button>
                        </div>

                        <div class="flex flex-col gap-1.5">
                            <label class="text-xs font-medium text-muted-foreground" for="provider">Provider</label>
                            <Popover.Root bind:open={providerOpen}>
                                <Popover.Trigger bind:ref={providerTriggerRef} id="provider" disabled={loading}>
                                    {#snippet child({ props }: { props: Record<string, unknown> })}
                                        <Button
                                            {...props}
                                            variant="outline"
                                            class="w-full justify-between text-sm h-9"
                                            role="combobox"
                                            aria-expanded={providerOpen}
                                            disabled={loading}
                                        >
                                            {selectedProviderName || (loading ? "Loading providers..." : providers.length === 0 ? "No providers available" : "Select a provider")}
                                            <ChevronsUpDownIcon class="size-4 opacity-50 shrink-0" />
                                        </Button>
                                    {/snippet}
                                </Popover.Trigger>
                                <Popover.Content class="p-0" style={providerTriggerRef ? `width: ${providerTriggerRef.clientWidth}px` : undefined}>
                                    <Command.Root>
                                        <Command.Input placeholder="Search provider..." />
                                        <Command.List>
                                            <Command.Empty>No provider found.</Command.Empty>
                                            <Command.Group>
                                                {#each providers as p (p.id)}
                                                    <Command.Item
                                                        value={p.id}
                                                        onSelect={() => {
                                                            localProvider = p.id;
                                                            localModel = "";
                                                            providerOpen = false;
                                                            closeAndFocus(providerTriggerRef);
                                                        }}
                                                    >
                                                        <CheckIcon class={cn("me-2 size-4", localProvider !== p.id && "text-transparent")} />
                                                        {p.name}
                                                    </Command.Item>
                                                {/each}
                                            </Command.Group>
                                        </Command.List>
                                    </Command.Root>
                                </Popover.Content>
                            </Popover.Root>
                        </div>

                        {#if currentProvider}
                            <div class="flex flex-col gap-1.5">
                                <label class="text-xs font-medium text-muted-foreground" for="model">Model</label>
                                <Popover.Root bind:open={modelOpen}>
                                <Popover.Trigger bind:ref={modelTriggerRef} id="model" disabled={availableModels.length === 0}>
                                    {#snippet child({ props }: { props: Record<string, unknown> })}
                                        <Button
                                            {...props}
                                            variant="outline"
                                            class="w-full justify-between text-sm h-9"
                                            role="combobox"
                                            aria-expanded={modelOpen}
                                            disabled={availableModels.length === 0}
                                        >
                                            {selectedModelName || (availableModels.length === 0 ? "No models available" : "Select a model")}
                                            <ChevronsUpDownIcon class="size-4 opacity-50 shrink-0" />
                                        </Button>
                                    {/snippet}
                                </Popover.Trigger>
                                    <Popover.Content class="p-0" style={modelTriggerRef ? `width: ${modelTriggerRef.clientWidth}px` : undefined}>
                                        <Command.Root>
                                            <Command.Input placeholder="Search model..." />
                                            <Command.List>
                                                <Command.Empty>No model found.</Command.Empty>
                                                <Command.Group>
                                                    {#each availableModels as m (m.id)}
                                                        <Command.Item
                                                            value={m.id}
                                                            onSelect={() => {
                                                                localModel = m.id;
                                                                modelOpen = false;
                                                                closeAndFocus(modelTriggerRef);
                                                            }}
                                                        >
                                                            <CheckIcon class={cn("me-2 size-4", localModel !== m.id && "text-transparent")} />
                                                            {m.name}{m.supports_tools ? " (tools)" : ""}
                                                        </Command.Item>
                                                    {/each}
                                                </Command.Group>
                                            </Command.List>
                                        </Command.Root>
                                    </Popover.Content>
                                </Popover.Root>
                            </div>

                            {#if needsApiKey}
                                <div class="flex flex-col gap-1.5">
                                    <label class="text-xs font-medium text-muted-foreground" for="api-key">API Key</label>
                                    <Input id="api-key" type="password" bind:value={localApiKey} placeholder="sk-..." />
                                    <p class="text-[10px] text-muted-foreground/50">
                                        Base URL: {currentProvider.api_url}
                                    </p>
                                </div>
                            {/if}
                        {/if}

                        {#if app.configured}
                            <div class="flex items-center gap-1.5 rounded-lg bg-emerald-500/5 border border-emerald-500/20 px-3 py-2">
                                <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-3.5 text-emerald-500 shrink-0">
                                    <path d="M20 6 9 17l-5-5" />
                                </svg>
                                <p class="text-xs text-emerald-600 dark:text-emerald-400">
                                    Configured: {app.selectedProvider} / {app.selectedModel}
                                </p>
                            </div>
                        {/if}

                        {#if !refreshing && providers.length === 0 && !loading}
                            <div class="flex items-center gap-1.5 rounded-lg bg-muted/50 px-3 py-2">
                                <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-3.5 text-muted-foreground/50 shrink-0">
                                    <line x1="1" y1="1" x2="23" y2="23" /><path d="M16.72 3.7A10 10 0 0 0 3.7 16.72" />
                                    <path d="M7.28 20.3A10 10 0 0 0 20.3 7.28" />
                                </svg>
                                <p class="text-xs text-muted-foreground/60">No providers found. Click refresh to fetch from network.</p>
                            </div>
                        {/if}
                    </Card>
                </div>
            {/if}

            <!-- ════════ Roles tab ════════ -->
            {#if activeTab === "roles"}
                <div class="h-full flex flex-col">
                    <div class="flex items-center justify-between mb-3 shrink-0">
                        <div>
                            <h2 class="text-base font-semibold">Roles</h2>
                            <p class="text-xs text-muted-foreground/60 mt-0.5">
                                {roles.length} role{roles.length !== 1 ? "s" : ""} defined
                            </p>
                        </div>
                        <Button size="xs" onclick={handleAddRole} disabled={!newRoleName.trim() || !newRoleDef.trim()}>
                            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-3"><path d="M5 12h14"/><path d="M12 5v14"/></svg>
                            New
                        </Button>
                    </div>

                    <div class="flex flex-1 min-h-0 gap-3">
                        <!-- Role list -->
                        <div class="w-48 shrink-0 overflow-y-auto space-y-0.5 rounded-md border border-border/50 bg-muted/10 p-1">
                            {#each roles as r (r.id)}
                                <button
                                    class={cn(
                                        "w-full flex items-center gap-2 px-2 py-1.5 rounded text-xs text-left transition-colors",
                                        editingId === r.id
                                            ? "bg-accent text-accent-foreground"
                                            : "text-muted-foreground hover:bg-muted/40 hover:text-foreground",
                                    )}
                                    onclick={() => {
                                        if (editingId !== r.id) startEdit(r);
                                    }}
                                >
                                    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-3 shrink-0 opacity-50"><path d="M12 2a4 4 0 1 0 0 8 4 4 0 0 0 0-8z"/><path d="M16 20v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/></svg>
                                    <span class="truncate">{r.name}</span>
                                </button>
                            {:else}
                                <div class="py-6 text-center text-xs text-muted-foreground/50">
                                    No roles yet
                                </div>
                            {/each}
                        </div>

                        <!-- Editor panel -->
                        <div class="flex-1 min-w-0 rounded-md border border-border/50 bg-card p-3">
                            {#if editingId}
                                {#each roles as r (r.id)}
                                    {#if editingId === r.id}
                                        <div class="flex flex-col h-full gap-2">
                                            <div class="flex items-center gap-2">
                                                <Input bind:value={editingName} placeholder="Role name" class="text-sm h-7 flex-1" />
                                                <div class="flex items-center gap-1 shrink-0">
                                                    <Button variant="ghost" size="icon-xs" onclick={saveEdit} title="Save" class="hover:text-emerald-500">
                                                        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-3.5"><path d="M20 6 9 17l-5-5"/></svg>
                                                    </Button>
                                                    <Button variant="ghost" size="icon-xs" onclick={cancelEdit} title="Cancel">
                                                        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-3.5"><path d="M18 6 6 18"/><path d="m6 6 12 12"/></svg>
                                                    </Button>
                                                </div>
                                            </div>
                                            <Textarea bind:value={editingDef} placeholder="System prompt / definition..." class="flex-1 min-h-0 resize-none text-xs" />
                                        </div>
                                    {/if}
                                {/each}
                            {:else}
                                <div class="flex flex-col items-center justify-center h-full text-center">
                                    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" class="size-8 text-muted-foreground/20 mb-2"><path d="M12 2a4 4 0 1 0 0 8 4 4 0 0 0 0-8z"/><path d="M16 20v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/></svg>
                                    <p class="text-xs text-muted-foreground/40">Select a role to edit</p>
                                </div>
                            {/if}
                        </div>
                    </div>

                    <!-- Inline new role form -->
                    <div class="flex items-center gap-2 mt-2 pt-2 border-t border-border/40 shrink-0">
                        <Input bind:value={newRoleName} placeholder="New role name" class="text-xs h-7 w-44" />
                        <Input bind:value={newRoleDef} placeholder="System prompt..." class="text-xs h-7 flex-1" />
                    </div>
                </div>
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
                <svg class="size-3.5 animate-spin mr-1" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                    <path d="M21 12a9 9 0 1 1-6.219-8.56"/>
                </svg>
            {/if}
            Save
        </Button>
    </div>
</div>
