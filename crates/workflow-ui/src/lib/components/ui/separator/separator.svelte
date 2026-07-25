<script lang="ts">
    import { cn } from "$lib/utils.js";
    import type { HTMLAttributes } from "svelte/elements";

    type Props = HTMLAttributes<HTMLHRElement> & {
        class?: string;
        orientation?: "horizontal" | "vertical";
        ref?: HTMLHRElement | null;
    };

    let { class: className, orientation = "horizontal", ref = $bindable(null), ...rest }: Props = $props();
</script>

{#if orientation === "horizontal"}
    <hr bind:this={ref} class={cn("shrink-0 bg-border h-px w-full", className)} {...rest} />
{:else}
    <hr bind:this={ref} class={cn("shrink-0 bg-border w-px h-full", className)} {...rest} />
{/if}
