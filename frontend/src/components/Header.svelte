<script lang="ts">
    import { onMount } from "svelte"
    import { invoke } from "@tauri-apps/api/core"
    import { isActive } from "@roxi/routify"
    import { Dashboard, Gear } from "radix-icons-svelte"

    let appVersion = ""

    onMount(async () => {
        try {
            appVersion = await invoke<string>("get_app_version")
        } catch (err) {
            console.error("failed to get app version:", err)
        }
    })
</script>

<header id="header">
    <div class="logo">
        <a href="/" title="Проект канала Хауди Хо!">
            <img src="/media/header-logo.png" alt="Jarvis Logo" />
        </a>
        <div>
            <h1><a href="/">JARVIS</a></h1>
            <h2>
                v{appVersion}
                <small class="beta-badge">BETA</small>
            </h2>
        </div>
    </div>

    <nav class="top-menu">
        <ul>
            <li>
                <a href="/commands" class:active={$isActive("/commands")}>
                    <Dashboard /> Команды
                </a>
            </li>
            <li>
                <a href="/settings" class:active={$isActive("/settings")}>
                    <Gear /> Настройки
                </a>
            </li>
        </ul>
    </nav>
</header>

<style lang="scss">
    .beta-badge {
        color: #8AC832;
        opacity: 0.9;
        font-size: 13px;
    }
</style>
