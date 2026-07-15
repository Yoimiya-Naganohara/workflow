<script lang="ts">
	import { invoke } from "@tauri-apps/api/core";
	import { listen } from "@tauri-apps/api/event";
	import { onMount } from "svelte";
	import { cn } from "$lib/utils.js";
	import { Button } from "$lib/components/ui/button";
	import { Textarea } from "$lib/components/ui/textarea";
	import { Input } from "$lib/components/ui/input";
	import { Card } from "$lib/components/ui/card";
	import { Separator } from "$lib/components/ui/separator";
	import { marked } from "marked";
	import * as Dialog from "$lib/components/ui/dialog";
	import * as Select from "$lib/components/ui/select";
	import TextBlock from "$lib/components/chat/text-block.svelte";
	import ToolCard from "$lib/components/chat/tool-card.svelte";
	import ErrorBlock from "$lib/components/chat/error-block.svelte";
	import ThinkingBlock from "$lib/components/chat/thinking-block.svelte";
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
		type: "user" | "text" | "thinking" | "tool" | "error";
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
		const unlistenWorkflow = listen<{ type: string; message?: string }>("workflow:event", (event) => {
			if (event.payload.type === "error") {
				error = event.payload.message ?? "runtime error";
				return;
			}
			pull();
		});
		return () => {
			unlistenWorkflow.then((unlisten) => unlisten());
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
			if (m.type === "text") {
				let html = "";
				try { html = String(marked.parse(m.text, { async: false })); } catch {}
				return { role: "text", text: m.text, html };
			}
			if (m.type === "tool") {
				return { role: "tool", text: m.text, html: "", result: m.result, status: m.result ? "done" : "running" };
			}
			return { role: m.type, text: m.text, html: "" };
		});
		if (selected != null && streamText[selected]) {
			const t = streamText[selected];
			if (t) {
				const last = items[items.length - 1];
			if (last?.role !== "text" || last.text !== t) {
				let html = "";
				try { html = String(marked.parse(t, { async: false })); } catch {}
				items.push({ role: "text", text: t, html, streaming: true });
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
			<Select.Root bind:value={newAgentRole} type="single">
				<Select.Trigger class="w-full text-xs h-8" id="new-role">
					{newAgentRole}
				</Select.Trigger>
				<Select.Content>
					{#each roles as r}
						<Select.Item value={r.name}>{r.name}</Select.Item>
					{/each}
				</Select.Content>
			</Select.Root>
		</div>
		<Card class="p-3 bg-muted/50">
			<p class="text-xs text-muted-foreground leading-relaxed">
				{roleDesc(newAgentRole)}
			</p>
		</Card>
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
			<Card class="p-3 space-y-1">
				<div class="flex items-center gap-2">
					<Brain class="size-3.5 text-muted-foreground" />
					<span class="text-sm font-medium">{r.name}</span>
				</div>
				<p class="text-xs text-muted-foreground">{r.definition}</p>
			</Card>
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
<div class="fixed inset-0 top-11 flex flex-row bg-background overflow-hidden">
	<!-- Sidebar -->
	<aside class="flex flex-col w-60 min-w-60 border-r border-border bg-muted overflow-hidden">
		<div class="flex items-center justify-between px-3 py-2 border-b border-border shrink-0">
			<div class="flex items-center gap-2">
				<Bot class="size-3.5 text-muted-foreground" />
				<span class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">Agents</span>
			</div>
			<div class="flex items-center gap-1">
				<span class="text-xs text-muted-foreground tabular-nums">{agents.length}</span>
				<Button variant="ghost" size="icon-xs" onclick={() => { loadRoles(); showNewAgent = true; }}>
					<Plus class="size-3" />
				</Button>
			</div>
		</div>
		<div class="flex-1 min-h-0 overflow-y-auto py-1.5 px-2">
			<div class="flex flex-col gap-0.5">
				{#each agents as a (a.id)}
					<div class="flex items-center gap-0.5 group">
						<Button
							variant="ghost"
							class={cn(
								"w-full justify-start gap-2 px-2 py-1.5 text-xs font-medium rounded-md",
								selected === a.id && "bg-accent text-accent-foreground border-l-2 border-primary rounded-s-none"
							)}
							onclick={() => selectAgent(a.id)}
						>
							<div class={cn("size-1.5 shrink-0 rounded-full", agentStatus(a))}></div>
							<div class="flex items-center gap-1.5 min-w-0">
								<span class="truncate">#{a.id}</span>
								<Badge variant="outline" class="text-[10px] px-1 py-px font-normal leading-none">{a.role}</Badge>
							</div>
						</Button>
						<Button
							variant="ghost"
							size="icon-xs"
							class="opacity-0 group-hover:opacity-100 shrink-0 hover:bg-destructive/10 text-muted-foreground hover:text-destructive transition-all duration-150"
							onclick={() => removeAgent(a.id)}
						>
							<Trash2 class="size-3" />
						</Button>
					</div>
				{:else}
					<div class="flex flex-col items-center gap-1.5 py-8 text-center">
						<Circle class="size-6 text-muted-foreground/30" />
						<p class="text-xs text-muted-foreground">No agents yet</p>
						<Button variant="outline" size="xs" onclick={() => { loadRoles(); showNewAgent = true; }} class="mt-1">
							<Plus class="size-3" /> Create
						</Button>
					</div>
				{/each}
			</div>
		</div>

		<Separator />

		<!-- Roles Section -->
		<div class="shrink-0">
			<Button
				variant="ghost"
				class="w-full justify-between px-3 py-2 text-xs font-semibold text-muted-foreground uppercase tracking-wider h-auto rounded-none"
				onclick={() => rolesExpanded = !rolesExpanded}
			>
				<div class="flex items-center gap-2">
					<Brain class="size-3.5" />
					<span>Roles</span>
				</div>
				<div class="flex items-center gap-1">
					<Button variant="ghost" size="icon-xs" onclick={(e) => { e.stopPropagation(); showRoles = true; }}>
						<Settings class="size-3" />
					</Button>
					{#if rolesExpanded}
						<ChevronDown class="size-3 transition-transform" />
					{:else}
						<ChevronRight class="size-3 transition-transform" />
					{/if}
				</div>
			</Button>
			{#if rolesExpanded}
				<div class="px-3 pb-2 flex flex-col gap-1">
					{#each roles as r}
						<div class="flex items-center gap-2 px-2 py-1 rounded">
							<div class="size-1.5 rounded-full bg-primary/40"></div>
							<span class="text-xs truncate">{r.name}</span>
						</div>
					{/each}
				</div>
			{/if}
		</div>
	</aside>

	<!-- Main Content -->
	<div class="flex flex-col flex-1 min-w-0 min-h-0 overflow-hidden">
		<!-- Tab Bar -->
		<div class="flex items-center gap-0.5 px-2 py-1 border-b border-border bg-card shrink-0">
			<Button
				variant="ghost"
				size="sm"
				class={cn("text-xs", currentTab === "chat" && "bg-accent text-accent-foreground")}
				onclick={() => currentTab = "chat"}
			>
				<MessageSquare class="size-3.5" />
				Chat
			</Button>
			<Button
				variant="ghost"
				size="sm"
				class={cn("text-xs", currentTab === "orchestrate" && "bg-accent text-accent-foreground")}
				onclick={() => currentTab = "orchestrate"}
			>
				<Blocks class="size-3.5" />
				Orchestrate
			</Button>
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
					<Card class="flex flex-col items-center gap-3 text-center py-12 px-8 max-w-xs border-dashed">
						<div class="size-12 rounded-full bg-muted flex items-center justify-center">
							<MessageSquare class="size-6 text-muted-foreground/30" />
						</div>
						<p class="text-sm font-medium text-muted-foreground/60">No messages yet</p>
						<p class="text-xs text-muted-foreground/40">Select an agent and send a message to begin.</p>
					</Card>
				</div>
			{:else}
				<div class="flex-1 min-h-0 overflow-y-auto" bind:this={scrollEl}>
					<div class="mx-auto max-w-3xl py-6 px-6">
						{#each renderedItems as item}
							<div class="mb-3 last:mb-0">
							{#if item.role === "text"}
								<TextBlock text={item.text} html={item.html} role="assistant" streaming={item.streaming ?? false} />
								{:else if item.role === "user"}
									<TextBlock text={item.text} html={item.html} role="user" />
								{:else if item.role === "thinking"}
									<ThinkingBlock text={item.text} />
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
				<Card class="shrink-0 mx-3 mb-2 px-3 py-2 bg-destructive/5 border-destructive/20 border-dashed">
					<p class="text-xs text-destructive">{error}</p>
				</Card>
			{/if}

			<div class="shrink-0 bg-card border-t border-border p-3">
				<form onsubmit={submit} class="mx-auto max-w-3xl flex gap-2 items-end">
					<div class="flex-1 relative">
						<Textarea
							bind:value={input}
							placeholder={selected != null ? "Type a message..." : "Select an agent first..."}
							class="min-h-9 max-h-32 resize-none text-sm pr-9"
							disabled={selected == null}
							onkeydown={(e) => {
								if (e.key === "Enter" && !e.shiftKey) {
									e.preventDefault();
									submit(e);
								}
							}}
						/>
						<div class="absolute right-1 bottom-1">
							<Button type="submit" disabled={!input.trim() || selected == null || loading} size="icon-sm">
								<Send class="size-3.5" />
							</Button>
						</div>
					</div>
				</form>
			</div>
			</div>
		{:else}
			<!-- Orchestrate Tab -->
			<div class="flex-1 flex items-center justify-center p-8">
				<Card class="flex flex-col items-center gap-3 text-center py-12 px-8 max-w-sm border-dashed">
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
				</Card>
			</div>
		{/if}
	</div>
</div>
