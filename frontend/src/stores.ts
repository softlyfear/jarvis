import { writable, get } from "svelte/store"
import { invoke } from "@tauri-apps/api/core"

// ### LISTENING STATE
// note: defaults to false since GUI doesn't have listening capability
export const isListening = writable(false)

// ### ASSISTANT VOICE
export const assistantVoice = writable("")

// load voice setting from db
async function loadVoiceSetting() {
    try {
        const voice = await invoke<string>("db_read", { key: "assistant_voice" })
        assistantVoice.set(voice)
    } catch (err) {
        console.error("failed to load voice setting:", err)
    }
}
loadVoiceSetting()

// ### APP INFO
// these are loaded once on startup
export const appInfo = writable({
    tgOfficialLink: "",
    feedbackLink: "",
    repositoryLink: "",
    logFilePath: ""
})

async function loadAppInfo() {
    try {
        const [tg, feedback, repo, logPath] = await Promise.all([
            invoke<string>("get_tg_official_link"),
            invoke<string>("get_feedback_link"),
            invoke<string>("get_repository_link"),
            invoke<string>("get_log_file_path")
        ])

        appInfo.set({
            tgOfficialLink: tg,
            feedbackLink: feedback,
            repositoryLink: repo,
            logFilePath: logPath
        })
    } catch (err) {
        console.error("failed to load app info:", err)
    }
}
loadAppInfo()
