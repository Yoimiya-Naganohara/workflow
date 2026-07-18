<script lang="ts">
    import "../app.css";
    import { onMount } from "svelte";
    import { ModeWatcher, toggleMode } from "mode-watcher";
    import { Button } from "$lib/components/ui/button";
    import { Sun, Moon, Bug } from "@lucide/svelte";
    import { TooltipProvider } from "$lib/components/ui/tooltip";
    import EventLog from "$lib/components/layout/event-log.svelte";
    import { emit } from "@tauri-apps/api/event";

    let { children } = $props();
    let showEventLog = $state(false);

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
                <div class="w-2"></div>
                <Button
                    variant="ghost"
                    size="icon-xs"
                    onclick={() => (showEventLog = !showEventLog)}
                >
                    <Bug class="size-3.5" />
                </Button>
                <Button variant="ghost" size="icon-xs" onclick={toggleMode}>
                    <Sun data-icon="inline-start" class="dark:hidden" />
                    <Moon data-icon="inline-start" class="hidden dark:block" />
                </Button>
            </div>
        </header>
        <div class="flex flex-1 min-w-0">
            {@render children()}
        </div>
        <EventLog open={showEventLog} />
    </div>
</TooltipProvider>
