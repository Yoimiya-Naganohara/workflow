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

<div class={cn("flex size-full items-center justify-center rounded-full bg-muted", className)} {...rest}>
    {#if children}
        {@render children()}
    {/if}
</div>
