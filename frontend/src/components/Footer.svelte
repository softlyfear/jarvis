<script lang="ts">
    const currentYear = new Date().getFullYear()
    const repoLink = "https://github.com/softlyfear/jarvis"
</script>

<footer id="footer">
    <p>© {currentYear}. Автор: <b>softlyfear</b></p>
    <p class="links">
        <a href={repoLink} target="_blank">
            <img src="/media/icons/github-logo.png" alt="GitHub" width="18px" />
            &nbsp;<span>github.com/softlyfear/jarvis</span>
        </a>
    </p>
    <!-- CC BY-NC-SA 4.0 requires crediting the original work -->
    <p class="origin">
        Основано на <a href="https://github.com/Priler/jarvis" target="_blank">Priler/jarvis</a> (Abraham Tugalov), лицензия CC BY-NC-SA 4.0
    </p>
</footer>

<style lang="scss">
    #footer {
        text-align: center;
        color: #6c6e71;
        font-size: 13px;
        font-weight: normal;
        line-height: 1.7em;
        margin-top: 15px;

        p {
            margin: 0;
            padding: 0;

            &.origin {
                font-size: 11px;
                margin-bottom: 15px;
            }

            &.links {
                margin-top: 5px;
                margin-bottom: 15px;

                &.last {
                    margin-top: -5px;
                }
            }
        }

        a {
            color: #555759!important;
            text-decoration: none;
            transition: 0.3s;
            
            & > span {
                color: #185876;
                border-bottom: 1px solid #185876;
                transition: 0.3s;
            }

            img {
                opacity: 0.5;
                transition: opacity 0.5s;
                margin-top: -3px;
            }

            &:hover {
                color: #777a7d!important;

                & > span {
                    color: #2A9CD0;
                }

                img {
                    opacity: 1;
                }
            }

            &.telegram-link {
                color: #185876;
                display: inline-block;

                &:hover {
                    color: #2A9CD0;
                    // background: url(/media/images/bg/bg24.gif);
                    // background-repeat: no-repeat;
                    // background-size: contain;
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
