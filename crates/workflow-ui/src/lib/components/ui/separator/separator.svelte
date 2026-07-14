<script lang="ts">
    import { cn } from "$lib/utils.js";
    import type { HTMLAttributes } from "svelte/elements";

    type Props = HTMLAttributes<HTMLHRElement> & {
        class?: string;
        orientation?: "horizontal" | "vertical";
    };

    let { class: className, orientation = "horizontal", ...rest }: Props = $props();
</script>

{#if orientation === "horizontal"}
    <hr class={cn("shrink-0 bg-border h-px w-full", className)} {...rest} />
{:else}
    <hr class={cn("shrink-0 bg-border w-px h-full", className)} {...rest} />
{/if}
