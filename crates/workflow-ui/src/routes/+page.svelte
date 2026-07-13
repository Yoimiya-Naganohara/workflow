<script lang="ts">
    import { invoke } from "@tauri-apps/api/core";
    import { listen } from "@tauri-apps/api/event";
    import { onMount } from "svelte";

    let agents = $state<any[]>([]);
    let selected = $state<number | null>(null);
    let messages = $state<any[]>([]);
    let input = $state("");

    async function pull() {
        const s = await invoke("snapshot");
        agents = s.agents;
        selected = s.selected;
        messages = s.messages;
    }

    onMount(() => {
        pull();
        const unlisten = listen("tick", () => pull());
        return () => {
            unlisten.then((f) => f());
        };
    });

    async function submit(e: Event) {
        e.preventDefault();
        if (!input.trim() || !selected) return;
        await invoke("send", { target: selected, text: input });
        input = "";
    }
</script>

<div class="app">
    <aside>
        {#each agents as a (a.id)}
            <button class:active={selected === a.id}>
                <b>#{a.id}</b>
                {a.role}
                {#if a.current_task}<small>{a.current_task}</small>{/if}
            </button>
        {/each}
    </aside>
    <main>
        <div class="log">
            {#each messages as m}
                <div class={m.role}>{m.text}</div>
            {/each}
        </div>
        <form onsubmit={submit}>
            <input bind:value={input} />
            <button type="submit">Send</button>
        </form>
    </main>
</div>

<style>
    :root {
        font-family: system-ui;
        font-size: 14px;
        color: #e0e0e0;
        background: #1a1a2e;
    }
    * {
        box-sizing: border-box;
        margin: 0;
        padding: 0;
    }
    .app {
        display: flex;
        height: 100vh;
    }
    aside {
        width: 220px;
        background: #16213e;
        border-right: 1px solid #0f3460;
        padding: 12px;
        overflow-y: auto;
    }
    aside button {
        display: block;
        width: 100%;
        text-align: left;
        background: none;
        border: none;
        color: inherit;
        padding: 8px;
        border-radius: 4px;
        cursor: pointer;
    }
    aside button.active {
        background: #0f3460;
    }
    aside small {
        display: block;
        font-size: 11px;
        color: #888;
    }
    main {
        flex: 1;
        display: flex;
        flex-direction: column;
    }
    .log {
        flex: 1;
        padding: 16px;
        overflow-y: auto;
    }
    .log div {
        margin-bottom: 8px;
        padding: 8px 12px;
        border-radius: 8px;
        max-width: 80%;
    }
    .user {
        margin-left: auto;
        background: #533483;
    }
    .assistant {
        background: #16213e;
    }
    .thinking {
        opacity: 0.5;
        font-style: italic;
    }
    .tool {
        color: #6a6;
        font-size: 12px;
    }
    .error {
        color: #a66;
    }
    form {
        display: flex;
        padding: 12px;
        gap: 8px;
        border-top: 1px solid #0f3460;
        background: #16213e;
    }
    input {
        flex: 1;
        background: #1a1a2e;
        border: 1px solid #0f3460;
        border-radius: 6px;
        padding: 8px;
        color: #e0e0e0;
    }
    button[type="submit"] {
        background: #533483;
        border: none;
        border-radius: 6px;
        padding: 8px 16px;
        color: #fff;
        cursor: pointer;
    }
</style>
