<script lang="ts">
	import { getContext } from "svelte";
	import { toggleMode } from "mode-watcher";
	import { Button } from "$lib/components/ui/button";
	import { Badge } from "$lib/components/ui/badge";
	import { ScrollArea } from "$lib/components/ui/scroll-area";
	import { Tooltip, TooltipContent, TooltipTrigger } from "$lib/components/ui/tooltip";
	import { Plus, MessageSquare, Brain, Settings as SettingsIcon, ChevronRight, ChevronDown, X, Loader2, CircleDot, AlertCircle, Sun, Moon, Bug } from "@lucide/svelte";
	import { cn, formatRole } from "$lib/utils.js";
	import type { AgentInfo, AgentId, AgentStatus } from "$lib/types";

	const eventLog = getContext<{ open: boolean; toggle: () => void }>("event-log");

	let {
		agents,
		selected,
		statuses,
		onSelect,
		onCreateClick,
		onRemoveAgent,
		onRolesClick,
		roles,
		rolesExpanded,
		onToggleRoles,
	}: {
		agents: AgentInfo[];
		selected: AgentId | null;
		statuses: Map<AgentId, AgentStatus>;
		onSelect: (id: AgentId) => void;
		onCreateClick: () => void;
		onRemoveAgent: (id: AgentId) => void;
		onRolesClick: () => void;
		roles: import("$lib/types").RoleInfo[];
		rolesExpanded: boolean;
		onToggleRoles: () => void;
	} = $props();

	function statusColor(status: AgentStatus): string {
		switch (status) {
			case "thinking":
			case "running-tool":
				return "bg-amber-500";
			case "responding":
				return "bg-emerald-500";
			case "error":
				return "bg-red-500";
			default:
				return "bg-muted-foreground/30";
		}
	}

	function statusIcon(status: AgentStatus) {
		switch (status) {
			case "thinking":
			case "running-tool":
				return Loader2;
			case "error":
				return AlertCircle;
			default:
				return CircleDot;
		}
	}

	function statusLabel(status: AgentStatus): string {
		switch (status) {
			case "thinking": return "Thinking";
			case "running-tool": return "Running tool";
			case "responding": return "Responding";
			case "error": return "Error";
			default: return "Idle";
		}
	}
</script>

<aside class="flex flex-col w-60 min-w-60 bg-transparent overflow-hidden shrink-0">
	<div class="flex items-center justify-between px-3 py-2 shrink-0">
		<div class="flex items-center gap-2">
			<MessageSquare class="size-3.5 text-muted-foreground" />
			<span class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">Agents</span>
		</div>
		<div class="flex items-center gap-1.5">
			<Badge variant="secondary" class="text-[10px] px-1.5 py-0 h-4 min-w-0">{agents.length}</Badge>
			<Button variant="ghost" size="icon-xs" onclick={onCreateClick} title="New agent">
				<Plus class="size-3" />
			</Button>
		</div>
	</div>

	<ScrollArea class="flex-1 min-h-0 no-scrollbar">
		<div class="py-1.5 px-2 flex flex-col gap-0.5">
			{#each agents as agent (agent.id)}
				{@const status = statuses.get(agent.id) ?? "idle"}
				{@const StatusIcon = statusIcon(status)}
				{@const isActive = status === "thinking" || status === "running-tool" || status === "responding"}
				<button
					class={cn(
						"w-full flex items-start gap-2 px-2 py-1.5 text-xs rounded-md text-left group relative",
						selected === agent.id
							? "bg-accent text-accent-foreground"
							: "hover:bg-muted/50",
					)}
					onclick={() => onSelect(agent.id)}
				>
					<div class="relative shrink-0 mt-0.5">
						<div class={cn("size-2 rounded-full", statusColor(status))}></div>
						{#if isActive}
							<div class={cn("absolute inset-0 size-2 rounded-full animate-ping opacity-75", statusColor(status))}></div>
						{/if}
					</div>
					<div class="flex-1 min-w-0">
						<div class="flex items-center gap-1">
							<span class="font-medium truncate">{formatRole(agent.role)}</span>
							<span class="text-[10px] text-muted-foreground/50 tabular-nums">#{agent.id}</span>
						</div>
						{#if agent.current_task}
							<Tooltip>
								<TooltipTrigger>
									<p class="text-[11px] text-muted-foreground/70 truncate mt-0.5 leading-tight">
										{agent.current_task}
									</p>
								</TooltipTrigger>
								<TooltipContent side="right" class="max-w-48">
									<p class="text-xs">{agent.current_task}</p>
								</TooltipContent>
							</Tooltip>
						{:else if status !== "idle"}
							<p class="text-[11px] text-muted-foreground/50 mt-0.5 leading-tight flex items-center gap-1">
								<StatusIcon class={cn("size-2.5", isActive && "animate-spin")} />
								{statusLabel(status)}
							</p>
						{/if}
					</div>
					<div class="absolute right-1 inset-y-0 flex items-center opacity-0 group-hover:opacity-100 transition-opacity">
						<Button
							variant="ghost"
							size="icon-xs"
							class="text-muted-foreground hover:text-destructive"
							onclick={(e) => { e.stopPropagation(); onRemoveAgent(agent.id); }}
							title="Remove agent"
						>
							<X class="size-3" />
						</Button>
					</div>
				</button>
			{:else}
				<div class="flex flex-col items-center gap-2 py-12 text-center px-4">
					<div class="size-10 rounded-full bg-muted/50 flex items-center justify-center">
						<MessageSquare class="size-4 text-muted-foreground/40" />
					</div>
					<p class="text-xs text-muted-foreground">No agents yet</p>
					<Button variant="outline" size="xs" onclick={onCreateClick}>
						<Plus class="size-3" /> New Agent
					</Button>
				</div>
			{/each}
		</div>
	</ScrollArea>

	<div class="shrink-0 border-t border-border/50">
		<button
			class="w-full flex items-center justify-between px-3 py-2 text-xs font-semibold text-muted-foreground uppercase tracking-wider hover:bg-muted/30 transition-colors"
			onclick={onToggleRoles}
		>
			<div class="flex items-center gap-2">
				<Brain class="size-3.5" />
				<span>Roles</span>
				<Badge variant="secondary" class="text-[10px] px-1.5 py-0 h-4 min-w-0">{roles.length}</Badge>
			</div>
			<div class="flex items-center gap-1">
				<Button variant="ghost" size="icon-xs" onclick={(e) => { e.stopPropagation(); onRolesClick(); }} title="Manage roles">
					<SettingsIcon class="size-3" />
				</Button>
				{#if rolesExpanded}
					<ChevronDown class="size-3 transition-transform" />
				{:else}
					<ChevronRight class="size-3 transition-transform" />
				{/if}
			</div>
		</button>
		{#if rolesExpanded}
			<div class="px-3 pb-2 flex flex-col gap-1 max-h-32 overflow-y-auto no-scrollbar">
				{#each roles as r}
					<Tooltip>
						<TooltipTrigger>
							<div class="flex items-center gap-2 px-2 py-1 rounded hover:bg-muted/30 transition-colors cursor-default">
								<div class="size-1.5 rounded-full bg-primary/40 shrink-0"></div>
								<span class="text-xs truncate">{r.name}</span>
							</div>
						</TooltipTrigger>
						<TooltipContent side="right" class="max-w-48">
							<p class="text-xs font-medium">{r.name}</p>
							<p class="text-[11px] text-muted-foreground mt-0.5">{r.definition}</p>
						</TooltipContent>
					</Tooltip>
				{/each}
			</div>
		{/if}
	</div>

	<div class="shrink-0 border-t border-border/50 px-3 py-2 flex items-center justify-between">
		<button
			onclick={toggleMode}
			class="flex items-center gap-2 text-xs text-muted-foreground/60 hover:text-foreground/80 transition-colors"
		>
			<Sun class="size-3.5 dark:hidden" />
			<Moon class="size-3.5 hidden dark:block" />
			<span class="dark:hidden">Light</span>
			<span class="hidden dark:inline">Dark</span>
		</button>
		<button
			onclick={eventLog.toggle}
			class="flex items-center gap-1.5 text-xs text-muted-foreground/60 hover:text-foreground/80 transition-colors"
			title="Event log"
		>
			<Bug class="size-3.5" />
			<span>Debug</span>
		</button>
	</div>
</aside>
