<script lang="ts">
    import { onMount } from "svelte"
    import { invoke } from "@tauri-apps/api/core"
    import { appInfo } from "@/stores"

    let authorName = ""
    let tgLink = ""
    let repoLink = ""

    const currentYear = new Date().getFullYear()

    appInfo.subscribe(info => {
        tgLink = info.tgOfficialLink
        repoLink = info.repositoryLink
    })

    onMount(async () => {
        try {
            authorName = await invoke<string>("get_author_name")
        } catch (err) {
            console.error("failed to get author name:", err)
        }
    })
</script>

<footer id="footer">
    <p>© {currentYear}. Автор проекта: {authorName}</p>
    <p class="links">
        <a href={tgLink} target="_blank" class="special-link">
            <img src="/media/icons/howdy-logo.png" alt="Telegram" width="20px" />
            &nbsp;&nbsp;Наш телеграм
        </a>
        канал.
        &nbsp;&nbsp;
        <a href={repoLink} target="_blank">
            <img src="/media/icons/github-logo.png" alt="GitHub" width="18px" />
            &nbsp;Github репозиторий
        </a>
        проекта.
    </p>
</footer>

<style lang="scss">
    #footer {
        text-align: center;
        color: #565759;
        font-size: 12px;
        font-weight: bold;
        line-height: 1.7em;
        margin-top: 15px;

        p {
            margin: 0;
            padding: 0;

            &.links {
                margin-top: 5px;
                margin-bottom: 15px;
            }
        }

        a {
            color: #185876;
            text-decoration: none;
            transition: opacity 0.5s;

            img {
                opacity: 0.5;
                transition: opacity 0.5s;
                margin-top: -4px;
            }

            &:hover {
                color: #2A9CD0;

                img {
                    opacity: 1;
                }
            }

            &.special-link {
                color: #941d92;
                display: inline-block;

                &:hover {
                    color: #FF07FC;
                    background: url(/media/images/bg/bg24.gif);
                    background-repeat: no-repeat;
                    background-size: contain;
                }
            }
        }
    }
</style>
