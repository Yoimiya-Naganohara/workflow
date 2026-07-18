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
            class="flex items-center justify-between h-8 shrink-0 bg-transparent select-none"
        >
            <div class="flex items-center gap-1 pr-20">
                <span class="text-xs font-medium text-muted-foreground ml-2">Workflow</span>
            </div>
        </header>
        <div class="flex flex-1 min-w-0">
            {@render children()}
        </div>
        <EventLog open={showEventLog} />
    </div>
</TooltipProvider>
