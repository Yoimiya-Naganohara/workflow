<script lang="ts">
	import { invoke } from "@tauri-apps/api/core";
	import { listen } from "@tauri-apps/api/event";
	import { onMount } from "svelte";
	import { cn } from "$lib/utils.js";
	import { Button } from "$lib/components/ui/button";
	import { Textarea } from "$lib/components/ui/textarea";
	import { Input } from "$lib/components/ui/input";
	import { marked } from "marked";
	import * as Dialog from "$lib/components/ui/dialog";
	import TextBlock from "$lib/components/chat/text-block.svelte";
	import ToolCard from "$lib/components/chat/tool-card.svelte";
	import ErrorBlock from "$lib/components/chat/error-block.svelte";
	import Badge from "$lib/components/ui/badge/badge.svelte";
	import {
		Send, Plus, Trash2, Settings, Brain, Blocks, Bot,
		MessageSquare, Circle, ChevronRight, ChevronDown
	} from "@lucide/svelte";

	interface AgentInfo {
		id: number;
		role: string;
		current_task: string | null;
	}

	interface UiMessage {
		role: "user" | "assistant" | "thinking" | "tool" | "error";
		text: string;
		result?: string | null;
	}

	interface RoleInfo {
		id: string;
		name: string;
		definition: string;
	}

	interface Snapshot {
		agents: AgentInfo[];
		selected: number | null;
		messages: UiMessage[];
	}

	let agents = $state<AgentInfo[]>([]);
	let selected = $state<number | null>(null);
	let messages = $state<UiMessage[]>([]);
	let input = $state("");
	let error = $state("");
	let loading = $state(false);

	// Dialogs
	let showNewAgent = $state(false);
	let showSettings = $state(false);
	let showRoles = $state(false);
	let rolesExpanded = $state(false);

	// New agent form
	let newAgentRole = $state("planner");
	let roles = $state<RoleInfo[]>([]);

	// Settings form
	let settingsApiKey = $state("");
	let settingsModel = $state("big-pickle");
	let settingsBaseUrl = $state("https://opencode.ai/zen/v1");

	// New role form
	let newRoleName = $state("");
	let newRoleDef = $state("");

	// Current tab
	type Tab = "chat" | "orchestrate";
	let currentTab = $state<Tab>("chat");

	async function loadRoles() {
		try {
			roles = (await invoke("get_roles")) as RoleInfo[];
		} catch (e) {
			console.error("load roles:", e);
		}
	}

	interface StreamDelta {
		id: number;
		text: string;
	}

	let streamText = $state<{ [key: number]: string }>({});

	async function pull(sel?: number | null) {
		try {
			const s = (await invoke("snapshot", { selected: sel ?? selected })) as Snapshot;
			agents = s.agents;
			if (s.selected !== null && s.selected !== undefined) selected = s.selected;
			messages = s.messages;
		} catch (e) {
			error = `snapshot: ${e}`;
		}
	}

	onMount(() => {
		loadRoles();
		pull(null);
		const unlistenStream = listen<StreamDelta>("stream", (e) => {
			streamText[e.payload.id] = e.payload.text;
		});
		const unlistenTick = listen("tick", () => {
			pull();
		});
		const interval = setInterval(pull, 2000);
		return () => {
			unlistenStream.then((fn) => fn());
			unlistenTick.then((fn) => fn());
			clearInterval(interval);
		};
	});

	async function submit(e?: Event) {
		e?.preventDefault();
		if (!input.trim() || selected == null) return;
		const text = input.trim();
		input = "";
		loading = true;
		if (selected != null) delete streamText[selected];
		try {
			const s = (await invoke("send", { target: selected, text })) as Snapshot;
			agents = s.agents;
			selected = s.selected;
			messages = s.messages;
			error = "";
		} catch (e) {
			error = `send: ${e}`;
		} finally {
			loading = false;
		}
	}

	async function createAgent() {
		try {
			agents = (await invoke("create_agent", { roleName: newAgentRole })) as AgentInfo[];
			showNewAgent = false;
		} catch (e) {
			error = `create agent: ${e}`;
		}
	}

	async function removeAgent(id: number) {
		try {
			agents = (await invoke("remove_agent", { id })) as AgentInfo[];
			if (selected === id) {
				selected = agents[0]?.id ?? null;
				await pull(selected);
			}
		} catch (e) {
			error = `remove agent: ${e}`;
		}
	}

	async function addRole() {
		if (!newRoleName.trim() || !newRoleDef.trim()) return;
		try {
			roles = (await invoke("add_role", { name: newRoleName.trim(), definition: newRoleDef.trim() })) as RoleInfo[];
			newRoleName = "";
			newRoleDef = "";
		} catch (e) {
			error = `add role: ${e}`;
		}
	}

	let scrollEl: HTMLDivElement | null = $state(null);
	let prevLen = $state(0);

	interface RenderedItem {
		role: string;
		text: string;
		html: string;
		result?: string | null;
		status?: "running" | "done";
		streaming?: boolean;
	}

	let renderedItems = $derived.by(() => {
		const items: RenderedItem[] = messages.map(m => {
			if (m.role === "assistant") {
				let html = "";
				try { html = String(marked.parse(m.text, { async: false })); } catch {}
				return { role: "assistant", text: m.text, html };
			}
			if (m.role === "tool") {
				return { role: "tool", text: m.text, html: "", result: m.result, status: m.result ? "done" : "running" };
			}
			return { role: m.role, text: m.text, html: "" };
		});
		if (selected != null && streamText[selected]) {
			const t = streamText[selected];
			if (t) {
				const last = items[items.length - 1];
				if (last?.role !== "assistant" || last.text !== t) {
					let html = "";
					try { html = String(marked.parse(t, { async: false })); } catch {}
					items.push({ role: "assistant", text: t, html, streaming: true });
				}
			}
		}
		return items;
	});

	$effect(() => {
		if (renderedItems.length > prevLen && scrollEl) {
			scrollEl.scrollTop = scrollEl.scrollHeight;
		}
		prevLen = renderedItems.length;
		if (selected != null && streamText[selected] && scrollEl) {
			scrollEl.scrollTop = scrollEl.scrollHeight;
		}
	});

	function selectAgent(id: number) {
		selected = id;
		pull(id);
	}

	function agentStatus(a: AgentInfo) {
		return a.current_task ? "bg-emerald-500" : "bg-muted-foreground/40";
	}

	function roleDesc(name: string) {
		const r = roles.find(r => r.name === name);
		return r ? r.definition : "";
	}
</script>

<!-- New Agent Dialog -->
<Dialog.Root open={showNewAgent} onOpenChange={(o) => showNewAgent = o}>
<Dialog.Content>
	<Dialog.Header>
		<Dialog.Title>New Agent</Dialog.Title>
		<Dialog.Description>Choose a role for the new agent.</Dialog.Description>
	</Dialog.Header>
	<div class="space-y-3">
		<div class="flex flex-col gap-1.5">
			<label class="text-xs font-medium text-muted-foreground" for="new-role">Role</label>
			<select
				id="new-role"
				bind:value={newAgentRole}
				class="flex h-8 w-full rounded-lg border border-input bg-background px-2.5 py-1 text-xs ring-offset-background focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
			>
				{#each roles as r}
					<option value={r.name}>{r.name}</option>
				{/each}
			</select>
		</div>
		<div class="rounded-lg bg-muted/50 border border-border/50 p-3">
			<p class="text-xs text-muted-foreground leading-relaxed">
				{roleDesc(newAgentRole)}
			</p>
		</div>
	</div>
	<Dialog.Footer>
		<Button variant="outline" onclick={() => showNewAgent = false}>Cancel</Button>
		<Button onclick={createAgent}><Bot class="size-3.5" /> Create Agent</Button>
	</Dialog.Footer>
</Dialog.Content>
</Dialog.Root>

<!-- Settings Dialog -->
<Dialog.Root open={showSettings} onOpenChange={(o) => showSettings = o}>
<Dialog.Content>
	<Dialog.Header>
		<Dialog.Title>Settings</Dialog.Title>
		<Dialog.Description>Configure your LLM provider.</Dialog.Description>
	</Dialog.Header>
	<div class="space-y-3">
		<div class="flex flex-col gap-1.5">
			<label class="text-xs font-medium text-muted-foreground" for="api-key">API Key</label>
			<Input id="api-key" type="password" bind:value={settingsApiKey} placeholder="sk-..." />
		</div>
		<div class="flex flex-col gap-1.5">
			<label class="text-xs font-medium text-muted-foreground" for="base-url">Base URL</label>
			<Input id="base-url" bind:value={settingsBaseUrl} placeholder="https://api.openai.com/v1" />
		</div>
		<div class="flex flex-col gap-1.5">
			<label class="text-xs font-medium text-muted-foreground" for="model">Model</label>
			<Input id="model" bind:value={settingsModel} placeholder="gpt-4" />
		</div>
		<p class="text-xs text-muted-foreground">Changes require restart to take effect.</p>
	</div>
	<Dialog.Footer>
		<Button onclick={() => showSettings = false}>Close</Button>
	</Dialog.Footer>
</Dialog.Content>
</Dialog.Root>

<!-- Roles Dialog -->
<Dialog.Root open={showRoles} onOpenChange={(o) => showRoles = o}>
<Dialog.Content class="sm:max-w-lg">
	<Dialog.Header>
		<Dialog.Title>Roles</Dialog.Title>
		<Dialog.Description>Manage agent roles and their system prompts.</Dialog.Description>
	</Dialog.Header>
	<div class="space-y-4 max-h-64 overflow-y-auto">
		{#each roles as r}
			<div class="rounded-lg border border-border p-3 space-y-1">
				<div class="flex items-center gap-2">
					<Brain class="size-3.5 text-muted-foreground" />
					<span class="text-sm font-medium">{r.name}</span>
				</div>
				<p class="text-xs text-muted-foreground">{r.definition}</p>
			</div>
		{/each}
	</div>
	<div class="border-t border-border pt-3 space-y-3">
		<p class="text-xs font-medium text-muted-foreground">Add New Role</p>
		<Input bind:value={newRoleName} placeholder="Role name" />
		<Textarea bind:value={newRoleDef} placeholder="System prompt / definition..." class="min-h-20 resize-none" />
		<Button onclick={addRole} class="w-full" size="sm">
			<Plus class="size-3.5" /> Add Role
		</Button>
	</div>
	<Dialog.Footer>
		<Button variant="outline" onclick={() => showRoles = false}>Close</Button>
	</Dialog.Footer>
</Dialog.Content>
</Dialog.Root>

<!-- Main Layout -->
<div style="position: fixed; top: 44px; left: 0; right: 0; bottom: 0; display: flex; flex-direction: row; background: var(--background); overflow: hidden;">
	<!-- Sidebar -->
	<aside style="display: flex; flex-direction: column; width: 240px; min-width: 240px; border-right: 1px solid var(--border); background: var(--muted); overflow: hidden;">
		<div style="display: flex; align-items: center; justify-content: space-between; padding: 8px 12px; border-bottom: 1px solid var(--border); flex-shrink: 0;">
			<div style="display: flex; align-items: center; gap: 8px;">
				<Bot style="width: 14px; height: 14px; color: var(--muted-foreground);" />
				<span style="font-size: 12px; font-weight: 600; color: var(--muted-foreground); text-transform: uppercase; letter-spacing: 0.05em;">Agents</span>
			</div>
			<div style="display: flex; align-items: center; gap: 4px;">
				<span style="font-size: 12px; color: var(--muted-foreground); font-variant-numeric: tabular-nums;">{agents.length}</span>
				<Button variant="ghost" size="icon-xs" onclick={() => { loadRoles(); showNewAgent = true; }}>
					<Plus style="width: 12px; height: 12px;" />
				</Button>
			</div>
		</div>
		<div style="flex: 1; min-height: 0; overflow-y: auto; padding: 6px 8px;">
			<div style="display: flex; flex-direction: column; gap: 2px;">
				{#each agents as a (a.id)}
					<div style="display: flex; align-items: center; gap: 2px;">
						<button
							class={cn(
								"flex items-center gap-2 flex-1 w-0 rounded-md px-2 py-1.5 text-left transition-colors",
								"hover:bg-accent hover:text-accent-foreground",
								"focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
								selected === a.id && "bg-accent text-accent-foreground border-l-2 border-primary rounded-l-none",
							)}
							onclick={() => selectAgent(a.id)}
						>
							<div class={cn("size-1.5 shrink-0 rounded-full", agentStatus(a))}></div>
							<div style="display: flex; align-items: center; gap: 6px; min-width: 0;">
								<span style="font-size: 12px; font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">#{a.id}</span>
								<Badge variant="outline" style="font-size: 10px; padding: 0 4px; font-weight: 400;">{a.role}</Badge>
							</div>
						</button>
						<button
							class="opacity-0 group-hover:opacity-100 shrink-0 rounded p-0.5 hover:bg-destructive/10 text-muted-foreground hover:text-destructive transition-opacity"
							onclick={() => removeAgent(a.id)}
						>
							<Trash2 style="width: 12px; height: 12px;" />
						</button>
					</div>
				{:else}
					<div style="display: flex; flex-direction: column; align-items: center; gap: 6px; padding: 32px 0; text-align: center;">
						<Circle style="width: 24px; height: 24px; color: var(--muted-foreground); opacity: 0.3;" />
						<p style="font-size: 12px; color: var(--muted-foreground);">No agents yet</p>
						<Button variant="outline" size="xs" onclick={() => { loadRoles(); showNewAgent = true; }} style="margin-top: 4px;">
							<Plus style="width: 12px; height: 12px;" /> Create
						</Button>
					</div>
				{/each}
			</div>
		</div>

		<!-- Roles Section -->
		<div style="border-top: 1px solid var(--border); flex-shrink: 0;">
			<button
				style="display: flex; align-items: center; justify-content: space-between; width: 100%; padding: 8px 12px; font-size: 12px; font-weight: 600; color: var(--muted-foreground); text-transform: uppercase; letter-spacing: 0.05em; hover:background: var(--accent); transition: colors; cursor: pointer; background: none; border: none;"
				onclick={() => rolesExpanded = !rolesExpanded}
			>
				<div style="display: flex; align-items: center; gap: 8px;">
					<Brain style="width: 14px; height: 14px;" />
					<span>Roles</span>
				</div>
				<div style="display: flex; align-items: center; gap: 4px;">
					<Button variant="ghost" size="icon-xs" onclick={(e) => { e.stopPropagation(); showRoles = true; }}>
						<Settings style="width: 12px; height: 12px;" />
					</Button>
					{#if rolesExpanded}
						<ChevronDown style="width: 12px; height: 12px;" />
					{:else}
						<ChevronRight style="width: 12px; height: 12px;" />
					{/if}
				</div>
			</button>
			{#if rolesExpanded}
				<div style="padding: 0 12px 8px; display: flex; flex-direction: column; gap: 4px;">
					{#each roles as r}
						<div style="display: flex; align-items: center; gap: 8px; padding: 4px 8px; border-radius: 4px;">
							<div style="width: 6px; height: 6px; border-radius: 50%; background: var(--primary); opacity: 0.4;"></div>
							<span style="font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{r.name}</span>
						</div>
					{/each}
				</div>
			{/if}
		</div>
	</aside>

	<!-- Main Content -->
	<div style="display: flex; flex-direction: column; flex: 1; min-width: 0; min-height: 0; overflow: hidden;">
		<!-- Tab Bar -->
		<div class="flex items-center gap-1 px-3 py-1.5 border-b border-border bg-card shrink-0">
			<button
				class={cn(
					"flex items-center gap-1.5 rounded-md px-2.5 py-1 text-xs font-medium transition-colors",
					currentTab === "chat" ? "bg-accent text-accent-foreground" : "text-muted-foreground hover:text-foreground hover:bg-accent/50"
				)}
				onclick={() => currentTab = "chat"}
			>
				<MessageSquare class="size-3.5" />
				Chat
			</button>
			<button
				class={cn(
					"flex items-center gap-1.5 rounded-md px-2.5 py-1 text-xs font-medium transition-colors",
					currentTab === "orchestrate" ? "bg-accent text-accent-foreground" : "text-muted-foreground hover:text-foreground hover:bg-accent/50"
				)}
				onclick={() => currentTab = "orchestrate"}
			>
				<Blocks class="size-3.5" />
				Orchestrate
			</button>
			<div class="flex-1"></div>
			<Button variant="ghost" size="icon-xs" onclick={() => showSettings = true}>
				<Settings class="size-3.5" />
			</Button>
		</div>

		<!-- Chat Tab -->
		{#if currentTab === "chat"}
			<div class="flex flex-col flex-1 min-h-0">
			{#if renderedItems.length === 0}
				<div class="flex-1 flex items-center justify-center">
					<div class="flex flex-col items-center gap-3 text-center py-16 max-w-xs">
						<div class="size-12 rounded-full bg-muted flex items-center justify-center">
							<MessageSquare class="size-6 text-muted-foreground/30" />
						</div>
						<p class="text-sm font-medium text-muted-foreground/60">No messages yet</p>
						<p class="text-xs text-muted-foreground/40">Select an agent and send a message to begin.</p>
					</div>
				</div>
			{:else}
				<div class="flex-1 min-h-0 overflow-y-auto" bind:this={scrollEl}>
					<div class="mx-auto max-w-3xl py-6 px-6">
						{#each renderedItems as item}
							<div class="mb-3 last:mb-0">
								{#if item.role === "assistant"}
									<TextBlock text={item.text} html={item.html} role="assistant" streaming={item.streaming ?? false} />
								{:else if item.role === "user"}
									<TextBlock text={item.text} html={item.html} role="user" />
								{:else if item.role === "thinking"}
									<div class="text-xs text-muted-foreground/60 italic leading-relaxed whitespace-pre-wrap">{item.text}</div>
								{:else if item.role === "tool"}
									<ToolCard name={item.text} result={item.result ?? undefined} status={item.status ?? "done"} />
								{:else if item.role === "error"}
									<ErrorBlock text={item.text} />
								{/if}
							</div>
						{/each}
					</div>
				</div>
			{/if}

			{#if error}
				<div class="shrink-0 px-4 py-1.5 bg-destructive/10 border-t border-destructive/20">
					<p class="text-xs text-destructive">{error}</p>
				</div>
			{/if}

			<div class="shrink-0 border-t border-border p-3">
				<form onsubmit={submit} class="mx-auto max-w-3xl flex gap-2 items-end">
					<Textarea
						bind:value={input}
						placeholder={selected != null ? "Type a message..." : "Select an agent first..."}
						class="min-h-9 max-h-32 resize-none text-sm"
						disabled={selected == null}
						onkeydown={(e) => {
							if (e.key === "Enter" && !e.shiftKey) {
								e.preventDefault();
								submit(e);
							}
						}}
					/>
					<Button type="submit" disabled={!input.trim() || selected == null || loading} class="shrink-0 mb-px">
						<Send class="size-3.5" />
					</Button>
				</form>
			</div>
			</div>
		{:else}
			<!-- Orchestrate Tab -->
			<div class="flex-1 flex items-center justify-center">
				<div class="flex flex-col items-center gap-3 text-center py-16 max-w-sm">
					<div class="size-12 rounded-full bg-muted flex items-center justify-center">
						<Blocks class="size-6 text-muted-foreground/40" />
					</div>
					<div>
						<p class="text-sm font-medium text-muted-foreground">Mission Orchestration</p>
						<p class="text-xs text-muted-foreground/60 mt-1">
							Design multi-agent workflows by defining a DAG of tasks with roles.
							Each task is assigned to an agent role and executed in dependency order.
						</p>
					</div>
					<div class="w-full mt-2 space-y-2">
						<Textarea
							placeholder="Describe your mission plan in natural language..."
							class="min-h-24 resize-none text-sm"
						/>
						<Button class="w-full" disabled>
							<Blocks class="size-3.5" /> Orchestrate
						</Button>
						<p class="text-[10px] text-muted-foreground/50">Requires LLM provider with API key configured.</p>
					</div>
				</div>
			</div>
		{/if}
	</div>
</div>
