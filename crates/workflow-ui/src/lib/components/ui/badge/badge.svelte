<script lang="ts">
    import { cn } from "$lib/utils.js";
    import type { HTMLAttributes } from "svelte/elements";
    import type { Snippet } from "svelte";

    type Props = HTMLAttributes<HTMLDivElement> & {
        children?: Snippet;
        variant?: "default" | "secondary" | "destructive" | "outline";
        class?: string;
    };

    let { children, variant = "default", class: className, ...rest }: Props = $props();
</script>

<div
    class={cn(
        "inline-flex items-center rounded-md border px-2.5 py-0.5 text-xs font-semibold transition-colors focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2",
        variant === "default" && "border-transparent bg-primary text-primary-foreground shadow",
        variant === "secondary" && "border-transparent bg-secondary text-secondary-foreground",
        variant === "destructive" && "border-transparent bg-destructive text-destructive-foreground shadow",
        variant === "outline" && "text-foreground",
        className,
    )}
    {...rest}
>
    {#if children}
        {@render children()}
    {/if}
</div>
