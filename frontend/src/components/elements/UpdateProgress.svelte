<script lang="ts">
    import { onMount } from "svelte"
    import { updateStatus, watchUpdate } from "@/stores"

    onMount(() => {
        watchUpdate()
    })

    const mb = (bytes: number) => (bytes / 1048576).toFixed(1)

    $: s = $updateStatus
    $: percent = s.total > 0 ? Math.min(100, Math.round((s.done / s.total) * 100)) : 0
    $: label =
        s.phase === "starting"
            ? `Обновление ${s.version} скачано. Установщик запущен, Джарвис перезапустится сам.`
            : s.total > 0
              ? `Скачиваю обновление ${s.version}: ${percent}% (${mb(s.done)} из ${mb(s.total)} МБ)`
              : `Скачиваю обновление ${s.version}… ${s.done > 0 ? mb(s.done) + " МБ" : ""}`
</script>

{#if s.phase === "downloading" || s.phase === "starting"}
    <div class="update-progress" role="status">
        <div class="update-label">{label}</div>
        <div class="update-track">
            <div
                class="update-bar"
                class:indeterminate={s.phase === "downloading" && s.total === 0}
                style="width: {s.phase === 'starting' ? 100 : s.total > 0 ? percent : 30}%"
            ></div>
        </div>
    </div>
{:else if s.phase === "failed"}
    <div class="update-progress failed" role="alert">Не удалось обновить: {s.error}</div>
{/if}

<style lang="scss">
    .update-progress {
        margin-top: 10px;
        font-size: 13px;
        color: #b8c4d6;
        max-width: 100%;
    }
    .update-progress.failed {
        color: #ff8a8a;
    }
    .update-label {
        margin-bottom: 6px;
        overflow-wrap: anywhere;
    }
    .update-track {
        height: 6px;
        border-radius: 3px;
        background: rgba(255, 255, 255, 0.1);
        overflow: hidden;
    }
    .update-bar {
        height: 100%;
        border-radius: 3px;
        background: #3fb950;
        transition: width 0.3s ease;
    }
    .update-bar.indeterminate {
        animation: slide 1.2s ease-in-out infinite;
    }
    @keyframes slide {
        from {
            transform: translateX(-100%);
        }
        to {
            transform: translateX(340%);
        }
    }
</style>
