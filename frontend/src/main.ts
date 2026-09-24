// ### STYLES
import "./css/main.scss"
import "./css/styles.scss"

// ### APP
import App from "./App.svelte"
import { installUiLogging } from "./lib/uilog"

installUiLogging()

const app = new App({
    target: document.getElementById("app")!
})

export default app
