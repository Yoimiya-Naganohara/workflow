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

	let {
		showSidebar,
		showGraph,
		showPins,
		onToggleSidebar,
		onToggleGraph,
		onTogglePins,
	}: {
		showSidebar: boolean;
		showGraph: boolean;
		showPins: boolean;
		onToggleSidebar: () => void;
		onToggleGraph: () => void;
		onTogglePins: () => void;
	} = $props();
</script>

<div class="flex items-center gap-0.5 px-2 py-1 border-b border-border bg-card shrink-0">
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
