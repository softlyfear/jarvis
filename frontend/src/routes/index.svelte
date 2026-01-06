<script lang="ts">
    import { onMount, onDestroy } from "svelte"
    import { invoke } from "@tauri-apps/api/core"
    import { Notification, Space, Button } from "@svelteuidev/core"
    import { InfoCircled } from "radix-icons-svelte"

    import ArcReactor from "@/components/elements/ArcReactor.svelte"
    import HDivider from "@/components/elements/HDivider.svelte"
    import Stats from "@/components/elements/Stats.svelte"
    import Footer from "@/components/Footer.svelte"

    import {
        isJarvisRunning,
        updateJarvisStats,
        jarvisState,
        ipcConnected,
        enableIpc,
        disableIpc
    } from "@/stores"

    let processRunning = false
    let launching = false

    // when process state changes, enable/disable IPC
    isJarvisRunning.subscribe((value) => {
        processRunning = value

        if (value) {
            // process is running, enable IPC connection
            enableIpc()
        } else {
            // process stopped, disable IPC (stops reconnect attempts)
            disableIpc()
        }
    })

    onMount(() => {
        document.body.classList.add("assist-page")
        updateJarvisStats()
    })

    onDestroy(() => {
        document.body.classList.remove("assist-page")
        disableIpc()
    })

    async function runAssistant() {
        launching = true
        try {
            await invoke("run_jarvis_app")

            // wait for startup
            setTimeout(async () => {
                await updateJarvisStats()
                launching = false
            }, 2500)
        } catch (err) {
            console.error("Failed to run jarvis-app:", err)
            launching = false
        }
    }
</script>

<HDivider />

{#if !processRunning}
    <Notification
        title="Внимание!"
        icon={InfoCircled}
        color="cyan"
        withCloseButton={false}
    >
        В данный момент ассистент не запущен.<br />
        Но вы всё еще можете изменять его настройки.<br />
        <br />

        <Button
            color="lime"
            radius="md"
            size="sm"
            uppercase
            ripple
            fullSize
            on:click={runAssistant}
            disabled={launching}
        >
            {launching ? "Запуск..." : "Запустить"}
        </Button>
    </Notification>
{:else}
    <ArcReactor />

    {#if !$ipcConnected}
        <Notification
            title="Подключение..."
            color="yellow"
            withCloseButton={false}
        >
            Устанавливается связь с ассистентом...
        </Notification>
    {/if}
{/if}

<HDivider noMargin />
<Stats />
<Footer />