<script lang="ts">
	import { goto } from "$app/navigation";
	import { Button } from "$lib/components/ui/button";
	import {
		PanelLeftClose,
		PanelLeftOpen,
		GitBranch,
		Pin,
		Settings,
	} from "@lucide/svelte";
	import SessionSelector from "./session-selector.svelte";
	import type { SessionMeta } from "$lib/types";

	let {
		showSidebar,
		showGraph,
		showPins,
		onToggleSidebar,
		onToggleGraph,
		onTogglePins,
		sessions = [],
		activeSessionId = null,
		defaultSessionName = "",
		onCreateSession,
		onSwitchSession,
		onDeleteSession,
		onRenameSession,
		onSetProject,
	}: {
		showSidebar: boolean;
		showGraph: boolean;
		showPins: boolean;
		onToggleSidebar: () => void;
		onToggleGraph: () => void;
		onTogglePins: () => void;
		sessions?: SessionMeta[];
		activeSessionId?: number | null;
		defaultSessionName?: string;
		onCreateSession?: (name: string) => void;
		onSwitchSession?: (id: number) => void;
		onDeleteSession?: (id: number) => void;
		onRenameSession?: (id: number, name: string) => Promise<boolean>;
		onSetProject?: (id: number, projectPath: string | null) => void;
	} = $props();
</script>

<div class="flex items-center gap-0.5 px-2 py-1 border-b border-border bg-card shrink-0">
	<SessionSelector
		{sessions}
		activeId={activeSessionId}
		defaultName={defaultSessionName}
		onCreate={(name: string) => onCreateSession?.(name)}
		onSwitch={(id: number) => onSwitchSession?.(id)}
		onDelete={(id: number) => onDeleteSession?.(id)}
		onRename={async (id: number, name: string) => onRenameSession?.(id, name) ?? false}
		onSetProject={(id: number, path: string | null) => onSetProject?.(id, path)}
	/>
	<Button
		variant="ghost"
		size="icon-xs"
		class="text-muted-foreground/50 hover:text-muted-foreground shrink-0"
		onclick={onToggleSidebar}
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
		onclick={onToggleGraph}
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
		onclick={onTogglePins}
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
