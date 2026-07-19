<script lang="ts">
    import { Bot, User } from "@lucide/svelte";

    let {
        text,
        html,
        role,
        streaming,
    }: {
        text: string;
        html: string;
        role: "user" | "assistant";
        streaming?: boolean;
    } = $props();

    function codeblock(node: HTMLElement) {
        let mo = new MutationObserver(() => {
            node.querySelectorAll("pre:not([data-copy])").forEach((pre) => {
                pre.setAttribute("data-copy", "true");
                pre.classList.add("group/pre", "relative");

                const btn = document.createElement("button");
                btn.className =
                    "absolute top-2 right-2 z-10 size-7 rounded-md flex items-center justify-center text-muted-foreground/50 hover:text-foreground hover:bg-background/80 opacity-0 group-hover/pre:opacity-100 transition-all";
                btn.innerHTML =
                    '<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/></svg>';
                btn.title = "Copy code";

                btn.addEventListener("click", async (e) => {
                    e.stopPropagation();
                    const code = pre.querySelector("code")?.textContent ?? "";
                    try {
                        await navigator.clipboard.writeText(code);
                        btn.innerHTML =
                            '<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"/></svg>';
                        btn.title = "Copied!";
                        setTimeout(() => {
                            btn.innerHTML =
                                '<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/></svg>';
                            btn.title = "Copy code";
                        }, 1500);
                    } catch {
                        /* clipboard unavailable */
                    }
                });

                pre.appendChild(btn);
            });
        });
        mo.observe(node, { childList: true, subtree: true });
        return {
            destroy() {
                mo.disconnect();
            },
        };
    }
</script>

<div class="flex items-start gap-3">
    {#if role === "user"}
        <div
            class="size-7 rounded-full bg-primary/10 flex items-center justify-center shrink-0 mt-0.5"
        >
            <User class="size-3.5 text-primary/70" />
        </div>
    {/if}
    <div class="min-w-0 flex-1">
        <div class="text-xs font-medium text-muted-foreground/60 mb-0.5">
            {role === "user" ? "You" : ""}
        </div>
        {#if role === "user"}
            <div
                class="text-sm leading-relaxed text-foreground/90 whitespace-pre-wrap break-words"
            >
                {text}
            </div>
        {:else if streaming}
            <div
                class="text-sm leading-relaxed text-foreground/90 whitespace-pre-wrap break-words"
            >
                {text}<span
                    class="inline-block w-[2px] h-[1.1em] bg-foreground animate-pulse ml-0.5 align-text-bottom"
                ></span>
            </div>
        {:else}
            <div
                use:codeblock
                class="text-sm leading-relaxed prose-sm dark:prose-invert max-w-none
				[&_code]:rounded [&_code]:bg-muted-foreground/10 [&_code]:px-1 [&_code]:py-0.5 [&_code]:text-xs [&_code]:font-mono
				[&_pre]:rounded-lg [&_pre]:border [&_pre]:border-border/40 [&_pre]:p-3 [&_pre]:text-xs [&_pre]:leading-relaxed [&_pre]:overflow-x-auto
				[&_pre_code]:bg-transparent [&_pre_code]:p-0
				[&_pre_>_code]:block
				.dark [&_pre]:border-border/60
				[&_blockquote]:border-l-2 [&_blockquote]:border-muted-foreground/20 [&_blockquote]:pl-3 [&_blockquote]:text-muted-foreground/70 [&_blockquote]:italic
				[&_ul]:list-disc [&_ul]:pl-4 [&_ol]:list-decimal [&_ol]:pl-4
				[&_a]:text-primary [&_a]:underline [&_a]:underline-offset-2 [&_a]:decoration-primary/30
				[&_h1]:text-base [&_h1]:font-semibold [&_h2]:text-sm [&_h2]:font-semibold [&_h3]:text-sm [&_h3]:font-medium
				[&_p]:my-0 [&_p+_p]:mt-2
				[&_:not(pre)_>_code]:before:content-['`'] [&_:not(pre)_>_code]:after:content-['`']
				[&_hr]:border-border/50
				[&_table]:w-full [&_table]:text-xs [&_th]:text-left [&_th]:font-medium [&_th]:text-muted-foreground [&_th]:pb-1 [&_td]:py-0.5 [&_td]:border-t [&_td]:border-border/30
			"
            >
                {@html html}
            </div>
        {/if}
    </div>
</div>
