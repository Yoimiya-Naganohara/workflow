<script lang="ts">
	import "../app.css";
	import { ModeWatcher, toggleMode } from "mode-watcher";
	import { Button } from "$lib/components/ui/button";
	import { Sun, Moon, Bug } from "@lucide/svelte";
	import EventLog from "$lib/components/layout/event-log.svelte";

	let { children } = $props();
	let showEventLog = $state(false);
</script>

<ModeWatcher />
<div class="flex h-dvh w-screen flex-col relative">
	<header class="flex items-center justify-between border-b border-border px-4 h-11 shrink-0 bg-card z-40">
		<div class="flex items-center gap-2">
			<div class="size-2 rounded-full bg-primary"></div>
			<span class="text-sm font-semibold tracking-tight">Agent Workflow</span>
		</div>
		<div class="flex items-center gap-1">
			<Button variant="ghost" size="icon-xs" onclick={() => showEventLog = !showEventLog}>
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
