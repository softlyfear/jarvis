<script lang="ts">
    import { Notification, Space } from "@svelteuidev/core"
    import { InfoCircled } from "radix-icons-svelte"

    import HDivider from "@/components/elements/HDivider.svelte"
    import Footer from "@/components/Footer.svelte"
    import { appInfo } from "@/stores"

    let tgLink = ""
    appInfo.subscribe(info => {
        tgLink = info.tgOfficialLink
    })
</script>

<Space h="xl" />

<Notification
    title="[404] Этот раздел еще находится в разработке!"
    icon={InfoCircled}
    color="blue"
    withCloseButton={false}
>
    Тут будет список команд + полноценный редактор команд.<br />
    Следите за обновлениями в <a href={tgLink} target="_blank">нашем телеграм канале</a>!
</Notification>

<div class="placeholder-image">
    <img src="/media/images/tenor.gif" alt="bruh" width="320px" />
</div>

<HDivider />
<Footer />

<style>
    .placeholder-image {
        text-align: center;
        margin-top: 25px;
    }
</style>
