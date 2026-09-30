<script context="module" lang="ts">
    export const load = ({ route }: { route: { url: string } }) => ({
        status: 404,
        props: { url: route.url },
    })
</script>

<script lang="ts">
    export let url = ""
</script>

<div class="missing-page">
    <h1>Страница не найдена</h1>
    <p>Не удалось открыть <code>{url}</code>.</p>
    <a href="/">Вернуться на главную</a>
</div>

<style>
    .missing-page { text-align: center; padding: 2rem; }
</style>
