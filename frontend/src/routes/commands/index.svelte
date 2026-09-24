<script lang="ts">
    import { onMount } from "svelte"
    import { invoke } from "@tauri-apps/api/core"
    import { Space, TextInput, Button } from "@svelteuidev/core"
    import { MagnifyingGlass } from "radix-icons-svelte"

    import HDivider from "@/components/elements/HDivider.svelte"
    import Footer from "@/components/Footer.svelte"

    interface CommandInfo { id: string; kind: string; phrases: string[] }
    interface CommandPack { pack: string; commands: CommandInfo[] }

    const PACK_NAMES: Record<string, string> = {
        apps: "Программы",
        games: "Игры",
        folders: "Папки",
        volume: "Звук",
        media: "Музыка и видео",
        system: "Компьютер",
        search: "Поиск в интернете",
        weather: "Погода",
        assistant: "Сам Джарвис",
        counter: "Счётчик (пример)",
    }
    const ORDER = ["apps", "games", "folders", "volume", "media", "system", "search", "weather", "assistant"]

    let packs: CommandPack[] = []
    let query = ""
    let loadError = ""

    onMount(async () => {
        try {
            const all = await invoke<CommandPack[]>("get_command_packs")
            packs = all.sort((a, b) => {
                const ia = ORDER.indexOf(a.pack), ib = ORDER.indexOf(b.pack)
                return (ia < 0 ? 99 : ia) - (ib < 0 ? 99 : ib)
            })
        } catch (err) {
            loadError = String(err)
            console.error("failed to load commands:", err)
        }
    })

    // {app} -> «…» so templates read naturally
    const pretty = (p: string) => p.replace(/\{[^}]+\}/g, "…")

    $: q = query.trim().toLowerCase()
    $: visible = packs
        .map((p) => ({
            ...p,
            commands: p.commands.filter((c) => !q || c.phrases.some((ph) => ph.toLowerCase().includes(q))),
        }))
        .filter((p) => p.commands.length > 0)

    async function openConfig() {
        try { await invoke("open_assistant_config") } catch (err) { console.error("open config:", err) }
    }
</script>

<Space h="xl" />

<div class="intro">
    <p>Скажите <b>«Джарвис»</b>, дождитесь отклика и произнесите команду. Вместо «…» — название: программы, игры, папки, число.</p>
    <p class="hint">Всё, чего нет в списке, понимает нейросеть Gemini (если в настройках есть ключ): вопросы, разговор, «удали вчерашний скриншот».</p>
</div>

<Space h="md" />
<TextInput icon={MagnifyingGlass} placeholder="Поиск по фразам" variant="filled" bind:value={query} />
<Space h="md" />

{#if loadError}
    <p class="error">Не удалось загрузить команды: {loadError}</p>
{/if}

{#each visible as pack (pack.pack)}
    <section class="pack">
        <h3>{PACK_NAMES[pack.pack] ?? pack.pack}</h3>
        {#each pack.commands as cmd (cmd.id)}
            <div class="cmd">
                {#each cmd.phrases.slice(0, 6) as ph}
                    <span class="phrase">{pretty(ph)}</span>
                {/each}
                {#if cmd.phrases.length > 6}
                    <span class="more">и ещё {cmd.phrases.length - 6}</span>
                {/if}
            </div>
        {/each}
    </section>
{/each}

{#if packs.length && !visible.length}
    <p class="hint">Ничего не найдено. Скорее всего, эту фразу поймёт нейросеть.</p>
{/if}

<Space h="lg" />
<div class="config">
    <p class="hint">Свои названия программ, игр и папок («телега» → Telegram) задаются в файле настроек.</p>
    <Button color="gray" radius="md" size="xs" uppercase on:click={openConfig}>Открыть файл настроек</Button>
</div>

<HDivider />
<Footer />

<style>
    .intro p { margin: 0 0 6px; color: #c9d1d9; font-size: 14px; }
    .hint { color: #8b949e; font-size: 13px; }
    .error { color: #ff7b72; }
    .pack { margin-bottom: 18px; }
    .pack h3 {
        margin: 0 0 8px;
        font-size: 14px;
        letter-spacing: 0.08em;
        text-transform: uppercase;
        color: #8ac832;
    }
    .cmd {
        display: flex;
        flex-wrap: wrap;
        gap: 6px;
        padding: 8px 10px;
        margin-bottom: 6px;
        border-radius: 8px;
        background: rgba(255, 255, 255, 0.04);
    }
    .phrase {
        padding: 2px 8px;
        border-radius: 12px;
        background: rgba(82, 254, 254, 0.1);
        color: #bff9ff;
        font-size: 13px;
    }
    .more { color: #8b949e; font-size: 12px; align-self: center; }
    .config { text-align: center; }
</style>
