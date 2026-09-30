import { writable } from "svelte/store"
import { getCurrentWindow } from "@tauri-apps/api/window"

// ### IPC STORES ###

export type JarvisState = "disconnected" | "idle" | "listening" | "processing"

export const jarvisState = writable<JarvisState>("disconnected")
export const ipcConnected = writable(false)
export const lastRecognizedText = writable("")
export const lastExecutedCommand = writable("")
export const lastError = writable("")

// orb visualizer: microphone level/spectrum (0..1) and synthesized speech
export const audioLevel = writable(0)
export const audioBands = writable<number[]>([])
export const speaking = writable(false)

// ### CONNECTION ###

const RECONNECT_DELAY = 5000

let ws: WebSocket | null = null
let reconnectTimer: ReturnType<typeof setTimeout> | null = null
let manualDisconnect = false
let enabled = false  // only connect when enabled
let ipcPort = 9712

export function enableIpc() {
    enabled = true
    manualDisconnect = false
    connectIpc()
}

export function disableIpc() {
    enabled = false
    disconnectIpc()
}

export function connectIpc(port: number = ipcPort) {
    if (!enabled || ws?.readyState === WebSocket.OPEN || ws?.readyState === WebSocket.CONNECTING) return
    ipcPort = port
    if (reconnectTimer) {
        clearTimeout(reconnectTimer)
        reconnectTimer = null
    }

    const socket = new WebSocket(`ws://127.0.0.1:${port}`)
    ws = socket

    socket.onopen = () => {
        if (ws !== socket) return
        ipcConnected.set(true)
        jarvisState.set("idle")
        console.log("[IPC] connected")
    }

    socket.onclose = () => {
        if (ws !== socket) return
        ws = null
        resetConnectionState()
        console.log("[IPC] disconnected")
        scheduleReconnect()
    }

    socket.onerror = (err) => {
        if (ws !== socket) return
        console.error("[IPC] error:", err)
    }

    socket.onmessage = (event) => {
        if (ws !== socket) return
        try {
            const msg = JSON.parse(event.data)
            handleEvent(msg)
        } catch (e) {
            console.error("[IPC] failed to parse message:", e)
        }
    }
}

function scheduleReconnect() {
    if (reconnectTimer || manualDisconnect || !enabled) return

    console.log(`IPC: Will retry in ${RECONNECT_DELAY / 1000}s...`)
    reconnectTimer = setTimeout(() => {
        reconnectTimer = null
        connectIpc()
    }, RECONNECT_DELAY)
}

export function disconnectIpc() {
    manualDisconnect = true

    if (reconnectTimer) {
        clearTimeout(reconnectTimer)
        reconnectTimer = null
    }

    if (ws) {
        const socket = ws
        ws = null
        socket.close()
    }

    resetConnectionState()
}

function resetConnectionState() {
    ipcConnected.set(false)
    jarvisState.set("disconnected")
    audioLevel.set(0)
    audioBands.set([])
    speaking.set(false)
}

// ### EVENT HANDLING ###

function handleEvent(data: any) {
    // ~30 per second, not worth logging
    if (data.event === "audio_level") {
        audioLevel.set(data.level || 0)
        audioBands.set(data.bands || [])
        return
    }

    console.log("IPC: Event", data.event, data)

    switch (data.event) {
        case "speaking":
            speaking.set(!!data.active)
            break

        case "wake_word_detected":
        case "listening":
            jarvisState.set("listening")
            break

        case "speech_recognized":
            lastRecognizedText.set(data.text || "")
            jarvisState.set("processing")
            break

        case "command_executed":
            lastExecutedCommand.set(data.id || "")
            break

        case "idle":
            jarvisState.set("idle")
            break

        case "error":
            lastError.set(data.message || "Unknown error")
            break

        case "started":
            jarvisState.set("idle")
            break

        case "stopping":
            jarvisState.set("disconnected")
            break

        case "pong":
            // connection verified
            break

        case "reveal_window":
            // bring window to foreground
            revealWindow()
            break
    }
}

// ### ACTIONS ###

export function sendAction(action: string, payload: Record<string, any> = {}) {
    if (ws?.readyState !== WebSocket.OPEN) {
        return false
    }

    ws.send(JSON.stringify({ action, ...payload }))
    return true
}

export function stopJarvisApp() {
    return sendAction("stop")
}

export function reloadCommands() {
    return sendAction("reload_commands")
}

export function sendIpcMessage(message: object): Promise<void> {
    return new Promise((resolve, reject) => {
        if (!ws || ws.readyState !== WebSocket.OPEN) {
            reject(new Error("IPC not connected"))
            return
        }

        try {
            ws.send(JSON.stringify(message))
            resolve()
        } catch (err) {
            reject(err)
        }
    })
}

export function sendTextCommand(text: string): boolean {
    return sendAction("text_command", { text })
}

async function revealWindow() {
    try {
        const window = getCurrentWindow()
        await window.show()
        await window.unminimize()
        await window.setFocus()
    } catch (e) {
        console.error("[IPC] Failed to reveal window:", e)
    }
}
