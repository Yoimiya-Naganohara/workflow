<script lang="ts">
    import { Pin, PinOff } from "@lucide/svelte";
    import { Button } from "$lib/components/ui/button";
    import { ScrollArea } from "$lib/components/ui/scroll-area";
    import SvelteMarkdown from "@humanspeak/svelte-markdown";
    import MarkdownCode from "$lib/markdown/markdown-code.svelte";
    import ThinkingBlock from "$lib/components/chat/thinking-block.svelte";
    import ToolCard from "$lib/components/chat/tool-card.svelte";
    import ErrorBlock from "$lib/components/chat/error-block.svelte";
    import type { PinnedMessage } from "$lib/types";

    let {
        messages,
        onUnpin,
        empty,
    }: {
        messages: PinnedMessage[];
        onUnpin: (pinId: number) => void;
        empty: boolean;
    } = $props();
</script>

{#if empty}
    <div class="flex-1 flex items-center justify-center">
        <div class="flex flex-col items-center gap-2 text-center px-4">
            <p class="text-xs text-muted-foreground/60">No pinned messages</p>
        </div>
    </div>
{:else}
    <ScrollArea class="flex-1 min-h-0 no-scrollbar">
        <div class="py-2 px-3 flex flex-col gap-2">
            {#each messages as pin (pin.id)}
                <div
                    class="group relative rounded-lg border border-border/40 bg-background/50 p-3"
                >
                    <div class="flex items-center justify-between mb-2">
                        <div
                            class="text-[10px] font-medium text-muted-foreground/40 uppercase tracking-wider flex items-center gap-2"
                        >
                            <span>{pin.type}</span>
                            {#if pin.agentRole}
                                <span>· {pin.agentRole}</span>
                            {/if}
                        </div>
                        <div
                            class="opacity-0 group-hover:opacity-100 transition-opacity"
                        >
                            <Button
                                variant="ghost"
                                size="icon-xs"
                                class="text-muted-foreground/40 hover:text-destructive"
                                onclick={() => onUnpin(pin.id)}
                                title="Unpin message"
                                aria-label="Unpin message"
                            >
                                <PinOff class="size-3" />
                            </Button>
                        </div>
                    </div>
                    <div class="text-xs leading-relaxed">
                        {#if pin.type === "user" || pin.type === "assistant"}
                            <div
                                class="prose-sm dark:prose-invert prose-code:before:content-none prose-code:after:content-none max-w-full"
                            >
                                <SvelteMarkdown source={pin.text}>
                                    {#snippet code({ lang, text: codeText })}
                                        <MarkdownCode
                                            lang={lang || "text"}
                                            text={codeText}
                                        />
                                    {/snippet}
                                </SvelteMarkdown>
                            </div>
                        {:else if pin.type === "thinking"}
                            <ThinkingBlock text={pin.text} />
                        {:else if pin.type === "tool"}
                            <ToolCard
                                name={pin.text}
                                result={pin.result ?? undefined}
                                status={pin.status ?? "done"}
                            />
                        {:else if pin.type === "error"}
                            <ErrorBlock text={pin.text} />
                        {/if}
                    </div>
                </div>
            {/each}
        </div>
    </ScrollArea>
{/if}
