<script lang="ts">
    import { onMount, onDestroy } from "svelte"
    import { invoke } from "@tauri-apps/api/core"

    import SearchBar from "@/components/elements/SearchBar.svelte"
    import VoiceOrb from "@/components/elements/VoiceOrb.svelte"
    import HDivider from "@/components/elements/HDivider.svelte"
    import Stats from "@/components/elements/Stats.svelte"
    import Footer from "@/components/Footer.svelte"
    
    import {
        isJarvisRunning,
        updateJarvisStats,
        enableIpc,
        disableIpc,
        translate,
        translations
    } from "@/stores"

    $: t = (key: string) => translate($translations, key)

    let processRunning = false
    let launching = false
    let stopping = false
    let wasRunning = false  // track previous state

    isJarvisRunning.subscribe((value) => {
        processRunning = value
        if (value) {
            enableIpc()
            wasRunning = true
        } else if (wasRunning) {
            // only disable if it was running before
            disableIpc()
            wasRunning = false
        }
    })

    onMount(() => {
        updateJarvisStats()
    })

    onDestroy(() => {
        disableIpc()
    })

    async function stopAssistant() {
        stopping = true
        try {
            await invoke("stop_jarvis_app")
        } catch (err) {
            console.error("Failed to stop jarvis-app:", err)
        }
        await updateJarvisStats()
        stopping = false
    }

    async function runAssistant() {
        launching = true
        try {
            await invoke("run_jarvis_app")
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

<div class="app-container assist-page">

    <div class="search search-section">
        <HDivider />
        <SearchBar />
    </div>

    <div class="reactor-section">
        <div class="reactor-wrapper" class:dimmed={!processRunning}>
            <VoiceOrb />
        </div>
        
        {#if !processRunning}
            <div class="offline-badge">
                <span class="offline-icon">⚠</span>
                <span class="offline-text">{t('assistant-not-running')}</span>
                <small>{t('assistant-offline-hint')}</small>
            </div>
            <button 
                class="start-button" 
                on:click={runAssistant}
                disabled={launching}
            >
                {launching ? t('btn-starting') : t('btn-start')}
            </button>
        {:else}
            <button class="stop-button" on:click={stopAssistant} disabled={stopping}>
                {stopping ? t('btn-stopping') : t('btn-stop')}
            </button>
        {/if}
    </div>

    <HDivider noMargin />
    <Stats />
    <Footer />
</div>