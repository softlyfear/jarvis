<script lang="ts">
    import { onMount, onDestroy } from "svelte"
    import { Notification, Space } from "@svelteuidev/core"
    import { InfoCircled } from "radix-icons-svelte"

    import SearchBar from "@/components/elements/SearchBar.svelte"
    import ArcReactor from "@/components/elements/ArcReactor.svelte"
    import HDivider from "@/components/elements/HDivider.svelte"
    import Stats from "@/components/elements/Stats.svelte"
    import Footer from "@/components/Footer.svelte"

    import { isListening } from "@/stores"

    let listening = false
    isListening.subscribe(value => {
        listening = value
    })

    onMount(() => {
        document.body.classList.add("assist-page")
    })

    onDestroy(() => {
        document.body.classList.remove("assist-page")
    })
</script>

<HDivider />

{#if !listening}
    <Notification
        title="Внимание!"
        icon={InfoCircled}
        color="cyan"
        withCloseButton={false}
    >
        В данный момент ассистент не прослушивает команды.<br />
        Пожалуйста, <a href="/settings">перейдите в настройки</a> и введите ключ Picovoice.
    </Notification>
{:else}
    <ArcReactor />
{/if}

<HDivider noMargin />
<Stats />
<Footer />
