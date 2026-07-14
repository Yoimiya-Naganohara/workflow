<script lang="ts">
    import { cn } from "$lib/utils.js";
    import type { HTMLImgAttributes } from "svelte/elements";

    type Props = HTMLImgAttributes & {
        class?: string;
    };

    let { class: className, ...rest }: Props = $props();
</script>

<img class={cn("aspect-square size-full", className)} {...rest} />
