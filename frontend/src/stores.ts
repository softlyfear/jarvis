import { writable } from "svelte/store"
import { invoke } from "@tauri-apps/api/core"

// ### RE-EXPORT IPC STORES
export {
    jarvisState,
    ipcConnected,
    lastRecognizedText,
    lastExecutedCommand,
    lastError,
    audioLevel,
    audioBands,
    speaking,
    connectIpc,
    enableIpc,
    disableIpc,
    disconnectIpc,
    sendAction,
    sendIpcMessage,
    sendTextCommand,
    stopJarvisApp,
    reloadCommands
} from "./lib/ipc"

// re-export i18n
export {
    translations,
    currentLanguage,
    translate,
    loadTranslations,
    setLanguage,
    loadLanguage,
    getSupportedLanguages
} from "./lib/i18n"

// ### RUNNING STATE
export const isJarvisRunning = writable(false)
export const jarvisRamUsage = writable(0)
export const jarvisCpuUsage = writable(0)

// ### ASSISTANT VOICE
export const assistantVoice = writable("")

// ### APP INFO
export const appInfo = writable({
    tgOfficialLink: "",
    feedbackLink: "",
    repositoryLink: "",
    boostySupportLink: "",
    patreonSupportLink: "",
    logFilePath: ""
})

// ### INIT FUNCTIONS (call these from a component)
export async function loadVoiceSetting() {
    try {
        const voice = await invoke<string>("db_read", { key: "assistant_voice" })
        assistantVoice.set(voice)
    } catch (err) {
        console.error("failed to load voice setting:", err)
    }
}

export async function loadAppInfo() {
    try {
        const [tg, feedback, repo, boosty, patreon, logPath] = await Promise.all([
            invoke<string>("get_tg_official_link"),
            invoke<string>("get_feedback_link"),
            invoke<string>("get_repository_link"),
            invoke<string>("get_boosty_link"),
            invoke<string>("get_patreon_link"),
            invoke<string>("get_log_file_path")
        ])

        appInfo.set({
            tgOfficialLink: tg,
            feedbackLink: feedback,
            repositoryLink: repo,
            boostySupportLink: boosty,
            patreonSupportLink: patreon,
            logFilePath: logPath
        })
    } catch (err) {
        console.error("failed to load app info:", err)
    }
}

export async function updateJarvisStats() {
    try {
        const stats = await invoke<{running: boolean, ram_mb: number, cpu_usage: number}>("get_jarvis_app_stats")
        isJarvisRunning.set(stats.running)
        jarvisRamUsage.set(stats.ram_mb)
        jarvisCpuUsage.set(stats.cpu_usage)
    } catch (err) {
        console.error("failed to get jarvis stats:", err)
    }
}

// polling manager
let statsInterval: ReturnType<typeof setInterval> | null = null

export function startStatsPolling(intervalMs = 5000) {
    if (statsInterval) return // already running
    
    updateJarvisStats()
    statsInterval = setInterval(updateJarvisStats, intervalMs)
}

export function stopStatsPolling() {
    if (statsInterval) {
        clearInterval(statsInterval)
        statsInterval = null
    }
}
// ### SELF-UPDATE
// the download runs in jarvis-gui itself: pages only show its progress, leaving one does not stop it
export interface UpdateStatus {
    phase: "idle" | "downloading" | "starting" | "failed"
    version: string
    done: number
    total: number
    error: string
}

export const updateStatus = writable<UpdateStatus>({ phase: "idle", version: "", done: 0, total: 0, error: "" })

let updateTimer: ReturnType<typeof setInterval> | null = null

async function pollUpdate() {
    try {
        const s = await invoke<UpdateStatus>("update_status")
        updateStatus.set(s)
        if (s.phase !== "downloading" && s.phase !== "starting" && updateTimer) {
            clearInterval(updateTimer)
            updateTimer = null
        }
    } catch (err) {
        console.error("update status:", err)
    }
}

// follow a running update (call on page mount and after pressing the button)
export async function watchUpdate() {
    await pollUpdate()
    if (!updateTimer) updateTimer = setInterval(pollUpdate, 400)
}

export async function startUpdate() {
    await invoke("install_update")
    await watchUpdate()
}
