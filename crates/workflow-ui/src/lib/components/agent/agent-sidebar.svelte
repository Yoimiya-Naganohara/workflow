<script lang="ts">
	import { Button } from "$lib/components/ui/button";
	import { Separator } from "$lib/components/ui/separator";
	import { Plus, Bot, Brain, Settings as SettingsIcon, ChevronRight, ChevronDown, Circle } from "@lucide/svelte";
	import { cn } from "$lib/utils.js";
	import AgentCard from "./agent-card.svelte";
	import type { AgentId } from "$lib/types";

	let {
		agents,
		selected,
		statuses,
		roles,
		rolesExpanded,
		onSelect,
		onRemove,
		onCreateClick,
		onRolesClick,
		onToggleRoles,
	}: {
		agents: import("$lib/types").AgentInfo[];
		selected: AgentId | null;
		statuses: Map<AgentId, import("$lib/types").AgentStatus>;
		roles: import("$lib/types").RoleInfo[];
		rolesExpanded: boolean;
		onSelect: (id: AgentId) => void;
		onRemove: (id: AgentId) => void;
		onCreateClick: () => void;
		onRolesClick: () => void;
		onToggleRoles: () => void;
	} = $props();
</script>

<aside class="flex flex-col w-60 min-w-60 border-r border-border bg-muted overflow-hidden shrink-0">
	<div class="flex items-center justify-between px-3 py-2 border-b border-border shrink-0">
		<div class="flex items-center gap-2">
			<Bot class="size-3.5 text-muted-foreground" />
			<span class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">Agents</span>
		</div>
		<div class="flex items-center gap-1">
			<span class="text-xs text-muted-foreground tabular-nums">{agents.length}</span>
			<Button variant="ghost" size="icon-xs" onclick={onCreateClick}>
				<Plus class="size-3" />
			</Button>
		</div>
	</div>

	<div class="flex-1 min-h-0 overflow-y-auto py-1.5 px-2">
		<div class="flex flex-col gap-0.5">
			{#each agents as a (a.id)}
				<AgentCard
					agent={a}
					selected={selected === a.id}
					status={statuses.get(a.id) ?? "idle"}
					onSelect={onSelect}
					onRemove={onRemove}
				/>
			{:else}
				<div class="flex flex-col items-center gap-1.5 py-8 text-center">
					<Circle class="size-6 text-muted-foreground/30" />
					<p class="text-xs text-muted-foreground">No agents yet</p>
					<Button variant="outline" size="xs" onclick={onCreateClick} class="mt-1">
						<Plus class="size-3" /> Create
					</Button>
				</div>
			{/each}
		</div>
	</div>

	<Separator />

	<div class="shrink-0">
		<Button
			variant="ghost"
			class="w-full justify-between px-3 py-2 text-xs font-semibold text-muted-foreground uppercase tracking-wider h-auto rounded-none"
			onclick={onToggleRoles}
		>
			<div class="flex items-center gap-2">
				<Brain class="size-3.5" />
				<span>Roles</span>
			</div>
			<div class="flex items-center gap-1">
				<Button variant="ghost" size="icon-xs" onclick={(e) => { e.stopPropagation(); onRolesClick(); }}>
					<SettingsIcon class="size-3" />
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
