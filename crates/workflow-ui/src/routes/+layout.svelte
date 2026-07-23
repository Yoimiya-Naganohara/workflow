<script lang="ts">
    import "../app.css";
    import { onMount, setContext } from "svelte";
    import { ModeWatcher } from "mode-watcher";
    import { TooltipProvider } from "$lib/components/ui/tooltip";
    import EventLog from "$lib/components/layout/event-log.svelte";
    import { emit } from "@tauri-apps/api/event";

    let { children } = $props();
    let showEventLog = $state(false);

    setContext("event-log", {
        get open() { return showEventLog; },
        toggle: () => showEventLog = !showEventLog,
    });

    onMount(() => {
        emit("decoration-page-load", {});
    });
</script>

<ModeWatcher />
<TooltipProvider>
    <div class="flex h-dvh w-screen flex-col relative">
        <header
            class="flex items-center justify-between h-8 shrink-0 bg-transparent select-none data-tauri-drag-region"
        >
            <div class="flex items-center gap-1 pr-20">
                <span class="text-xs font-semibold text-muted-foreground ml-2 tracking-wide">Workflow</span>
            </div>
            <div class="flex items-center gap-2 mr-2">
                <span class="text-[10px] text-muted-foreground/30 font-mono tabular-nums">v0.1</span>
            </div>
        </header>
        <div class="flex flex-1 min-w-0" style="transition: var(--transition-theme)">
            {@render children()}
        </div>
        <EventLog open={showEventLog} />
    </div>
</TooltipProvider>
