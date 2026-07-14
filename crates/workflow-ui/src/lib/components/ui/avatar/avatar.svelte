<script lang="ts">
    import { cn } from "$lib/utils.js";
    import type { HTMLAttributes } from "svelte/elements";
    import type { Snippet } from "svelte";

    type Props = HTMLAttributes<HTMLDivElement> & {
        children?: Snippet;
        class?: string;
    };

    let { children, class: className, ...rest }: Props = $props();
</script>

<div class={cn("relative flex size-10 shrink-0 overflow-hidden rounded-full", className)} {...rest}>
    {#if children}
        {@render children()}
    {/if}
</div>
