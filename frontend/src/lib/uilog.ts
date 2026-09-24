// Sends UI events to gui-log.txt (via the ui_log Tauri command) so bug reports carry context:
// clicks, route changes, console errors, uncaught exceptions.
import { invoke } from "@tauri-apps/api/core"

type Level = "info" | "warn" | "error"

export function uiLog(level: Level, message: string) {
    invoke("ui_log", { level, message }).catch(() => {
        // outside Tauri (vite dev in a browser) there is no backend
    })
}

function describe(el: Element | null): string | null {
    const target = el?.closest("button, a, [role=tab], input[type=checkbox], select, .voice-option")
    if (!target) return null
    const text = (target.textContent || (target as HTMLInputElement).value || "").trim().replace(/\s+/g, " ")
    const label = target.getAttribute("aria-label") || target.getAttribute("title") || ""
    return `${target.tagName.toLowerCase()} "${(label || text).slice(0, 80)}"`
}

export function installUiLogging() {
    uiLog("info", `UI started, ${navigator.userAgent}`)

    document.addEventListener("click", (e) => {
        const what = describe(e.target as Element)
        if (what) uiLog("info", `click ${what} on ${location.pathname}`)
    }, true)

    document.addEventListener("change", (e) => {
        const el = e.target as HTMLInputElement
        if (el?.tagName === "SELECT") uiLog("info", `select "${el.value}" on ${location.pathname}`)
    }, true)

    let lastPath = location.pathname
    setInterval(() => {
        if (location.pathname !== lastPath) {
            lastPath = location.pathname
            uiLog("info", `route ${lastPath}`)
        }
    }, 500)

    window.addEventListener("error", (e) => uiLog("error", `uncaught: ${e.message} at ${e.filename}:${e.lineno}`))
    window.addEventListener("unhandledrejection", (e) => uiLog("error", `unhandled rejection: ${String(e.reason)}`))

    const origError = console.error
    console.error = (...args: unknown[]) => {
        uiLog("error", args.map((a) => (a instanceof Error ? a.message : typeof a === "string" ? a : JSON.stringify(a))).join(" "))
        origError(...args)
    }
}
