// The settings field shows one gateway key at a time. A pasted key can identify
// its gateway; ordinary edits must stay in the field that is currently shown.

export type Gateway = "openrouter" | "polza"

export interface GatewayKeys {
    gateway: Gateway
    openrouterKey: string
    polzaKey: string
}

export function assignGatewayKey(state: GatewayKeys, value: string, pasted: boolean): GatewayKeys {
    const key = value.replace(/\s+/g, "")
    let gateway = state.gateway
    // A partial edit of "sk-or-…" can pass through "sk-…" . Switching on that
    // prefix would replace the other gateway's saved key.
    if (pasted && key.length >= 20) {
        if (key.startsWith("sk-or-")) gateway = "openrouter"
        else if (key.startsWith("sk-")) gateway = "polza"
    }
    if (gateway === "polza") return { ...state, gateway, polzaKey: value }
    return { ...state, gateway, openrouterKey: value }
}
