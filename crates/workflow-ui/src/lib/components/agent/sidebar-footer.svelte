<script lang="ts">
	import { getContext } from "svelte";
	import { toggleMode } from "mode-watcher";
	import { Sun, Moon, Bug } from "@lucide/svelte";

	const eventLog = getContext<{ open: boolean; toggle: () => void }>("event-log");
</script>

<div class="shrink-0 px-3 py-2 flex items-center justify-between">
	<button
		onclick={toggleMode}
		class="flex items-center gap-2 text-xs text-muted-foreground/60 hover:text-foreground/80 transition-colors"
		aria-label="Toggle color theme"
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
		aria-label="Toggle event log"
	>
		<Bug class="size-3.5" />
		<span>Debug</span>
	</button>
</div>
