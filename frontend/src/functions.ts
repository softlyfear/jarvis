import { invoke } from "@tauri-apps/api/core"

// ### UTILITY FUNCTIONS

export function capitalizeFirstLetter(str: string): string {
    if (!str) return ""
    return str.charAt(0).toUpperCase() + str.slice(1)
}

export function showInExplorer(path: string): void {
    invoke("show_in_folder", { path })
        .catch(err => console.error("failed to open explorer:", err))
}

// ### LISTENER FUNCTIONS
// removed since gui now doesn't handle listening

export function startListening(): void {
    // disabled in GUI - listening is handled by the tray app
    console.log("[gui] listening not available in settings app")
}

export function stopListening(callback?: () => void): void {
    // disabled in GUI - just call the callback if provided
    console.log("[gui] listening not available in settings app")
    if (callback) callback()
}
