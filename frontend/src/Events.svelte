<script lang="ts">
    import { onMount, onDestroy } from "svelte"
    import { listen } from "@tauri-apps/api/event"
    import { invoke } from "@tauri-apps/api/core"
    import { assistantVoice } from "@/stores"

    let voiceVal = "jarvis-og"
    const unsubscribeVoice = assistantVoice.subscribe(value => {
        voiceVal = value || "jarvis-og"
    })

    let destroyed = false
    const unlisten: (() => void)[] = []
    function registerListener(stop: () => void) {
        if (destroyed) stop()
        else unlisten.push(stop)
    }
    onDestroy(() => {
        destroyed = true
        unsubscribeVoice()
        unlisten.forEach(stop => stop())
    })

    onMount(() => {
        const register = async () => {
        // audio playback event
        registerListener(await listen<{ data: string }>("audio-play", async (event) => {
            const voice = voiceVal || "jarvis-remake"
            const filename = `sound/${voice}/${event.payload.data}.wav`

            try {
                await invoke("play_sound", { filename, sleep: true })
            } catch (err) {
                console.error("failed to play sound:", err)
            }
        }))

        // assistant state events
        registerListener(await listen("assistant-greet", () => {
            document.getElementById("arc-reactor")?.classList.add("active")
        }))

        registerListener(await listen("assistant-waiting", () => {
            document.getElementById("arc-reactor")?.classList.remove("active")
        }))
        }
        register().catch(err => console.error("Failed to register events:", err))
    })
</script>
