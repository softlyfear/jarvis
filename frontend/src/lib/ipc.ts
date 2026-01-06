import { writable, get } from "svelte/store"

// ### IPC STORES ###

export type JarvisState = "disconnected" | "idle" | "listening" | "processing"

export const jarvisState = writable<JarvisState>("disconnected")
export const ipcConnected = writable(false)
export const lastRecognizedText = writable("")
export const lastExecutedCommand = writable("")
export const lastError = writable("")

// ### CONNECTION ###

const IPC_URL = "ws://127.0.0.1:9712"
const RECONNECT_DELAY = 5000

let ws: WebSocket | null = null
let reconnectTimer: ReturnType<typeof setTimeout> | null = null
let manualDisconnect = false
let enabled = false  // only connect when enabled

export function enableIpc() {
    enabled = true
    connectIpc()
}

export function disableIpc() {
    enabled = false
    disconnectIpc()
}

export function connectIpc() {
    if (!enabled) {
        console.log("IPC: Not enabled, skipping connection")
        return
    }

    if (ws?.readyState === WebSocket.OPEN || ws?.readyState === WebSocket.CONNECTING) {
        return
    }

    manualDisconnect = false

    console.log("IPC: Connecting to", IPC_URL)

    try {
        ws = new WebSocket(IPC_URL)

        ws.onopen = () => {
            console.log("IPC: Connected")
            ipcConnected.set(true)
            jarvisState.set("idle")
            sendAction("ping")
        }

        ws.onclose = (event) => {
            console.log("IPC: Disconnected", event.code)
            ipcConnected.set(false)
            jarvisState.set("disconnected")
            ws = null

            if (!manualDisconnect && enabled) {
                scheduleReconnect()
            }
        }

        ws.onerror = () => {
            // error is handled in onclose, just suppress console spam
        }

        ws.onmessage = (event) => {
            try {
                const data = JSON.parse(event.data)
                handleEvent(data)
            } catch (e) {
                console.error("IPC: Failed to parse message:", event.data, e)
            }
        }
    } catch (e) {
        // suppress errors when server isn't running
        if (enabled) {
            scheduleReconnect()
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
        ws.close()
        ws = null
    }

    ipcConnected.set(false)
    jarvisState.set("disconnected")
}

// ### EVENT HANDLING ###

function handleEvent(data: any) {
    console.log("IPC: Event", data.event, data)

    switch (data.event) {
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