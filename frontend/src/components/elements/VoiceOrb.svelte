<script lang="ts">
    // Glowing orb that breathes when idle, follows the microphone while listening
    // (old-school equalizer bars with falling peaks), swirls while thinking and
    // pulses while the assistant speaks.
    import { onMount, onDestroy } from "svelte"
    import { jarvisState, audioLevel, audioBands, speaking } from "@/stores"

    export let size = 320

    const BARS = 48
    const COLORS: Record<string, [number, number, number]> = {
        disconnected: [110, 122, 135],
        idle: [60, 220, 255],
        listening: [120, 245, 255],
        processing: [170, 125, 255],
        speaking: [255, 185, 80],
    }

    let canvas: HTMLCanvasElement
    let frame = 0

    // latest data from IPC
    let level = 0
    let bands: number[] = []
    let state = "disconnected"
    let isSpeaking = false
    const unsubs = [
        audioLevel.subscribe((v) => (level = v)),
        audioBands.subscribe((v) => (bands = v)),
        jarvisState.subscribe((v) => (state = v)),
        speaking.subscribe((v) => (isSpeaking = v)),
    ]

    // animated values
    let smoothLevel = 0
    let color: [number, number, number] = [...COLORS.disconnected]
    const bars = new Array(BARS).fill(0)
    const peaks = new Array(BARS).fill(0)
    let spin = 0

    function mode(): string {
        if (state === "disconnected") return "disconnected"
        if (isSpeaking) return "speaking"
        return state
    }

    // value 0..1 for bar i at time t
    function target(i: number, t: number, m: string): number {
        // mirror around the top so the picture is symmetric
        const half = BARS / 2
        const pos = Math.abs(i - half) / half // 1 at the top (low bands), 0 at the bottom
        switch (m) {
            case "listening": {
                if (bands.length) {
                    const b = Math.min(bands.length - 1, Math.floor(pos * bands.length))
                    return bands[b]
                }
                return 0.08 + 0.05 * Math.sin(t * 3 + i)
            }
            case "processing":
                return 0.25 + 0.22 * Math.sin((i / BARS) * Math.PI * 6 + t * 5)
            case "speaking": {
                const env = 0.3 + 0.35 * Math.abs(Math.sin(t * 6.3) * Math.sin(t * 2.2 + 0.5))
                return env * (0.55 + 0.45 * Math.sin(pos * 9 + t * 11))
            }
            case "idle":
                return 0.06 + 0.04 * (1 + Math.sin(t * 1.4 + pos * 4))
            default:
                return 0.02
        }
    }

    function draw(now: number) {
        frame = requestAnimationFrame(draw)
        const ctx = canvas?.getContext("2d")
        if (!ctx) return

        const dpr = window.devicePixelRatio || 1
        if (canvas.width !== size * dpr) {
            canvas.width = size * dpr
            canvas.height = size * dpr
        }
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
        ctx.clearRect(0, 0, size, size)

        const t = now / 1000
        const m = mode()
        const c = size / 2

        // colour and loudness ease towards their targets
        const want = COLORS[m] || COLORS.idle
        for (let k = 0; k < 3; k++) color[k] += (want[k] - color[k]) * 0.06
        const rawLevel =
            m === "listening" ? level :
            m === "speaking" ? 0.35 + 0.3 * Math.abs(Math.sin(t * 6.3)) :
            m === "processing" ? 0.25 : 0.05 + 0.03 * Math.sin(t * 1.4)
        smoothLevel += (rawLevel - smoothLevel) * (rawLevel > smoothLevel ? 0.35 : 0.08)

        const [r, g, b] = color.map(Math.round)
        const rgba = (a: number) => `rgba(${r}, ${g}, ${b}, ${a})`
        const radius = size * (0.19 + 0.07 * smoothLevel)

        // soft halo
        const halo = ctx.createRadialGradient(c, c, radius * 0.6, c, c, size * 0.5)
        halo.addColorStop(0, rgba(0.18 + 0.25 * smoothLevel))
        halo.addColorStop(1, rgba(0))
        ctx.fillStyle = halo
        ctx.fillRect(0, 0, size, size)

        // equalizer bars with falling peaks
        const inner = radius + size * 0.05
        const maxLen = size * 0.13
        ctx.lineCap = "round"
        for (let i = 0; i < BARS; i++) {
            const v = Math.max(0, Math.min(1, target(i, t, m)))
            bars[i] += (v - bars[i]) * (v > bars[i] ? 0.55 : 0.12)
            peaks[i] = Math.max(peaks[i] - 0.012, bars[i])

            const angle = (i / BARS) * Math.PI * 2 - Math.PI / 2
            const cos = Math.cos(angle), sin = Math.sin(angle)
            const len = 2 + bars[i] * maxLen

            ctx.strokeStyle = rgba(0.35 + 0.6 * bars[i])
            ctx.lineWidth = Math.max(2, size * 0.011)
            ctx.beginPath()
            ctx.moveTo(c + cos * inner, c + sin * inner)
            ctx.lineTo(c + cos * (inner + len), c + sin * (inner + len))
            ctx.stroke()

            const pr = inner + 4 + peaks[i] * maxLen
            ctx.fillStyle = rgba(0.9)
            ctx.beginPath()
            ctx.arc(c + cos * pr, c + sin * pr, Math.max(1.2, size * 0.005), 0, Math.PI * 2)
            ctx.fill()
        }

        // thin rotating ring
        spin += m === "processing" ? 0.05 : m === "listening" ? 0.02 : 0.006
        ctx.save()
        ctx.translate(c, c)
        ctx.rotate(spin)
        ctx.setLineDash([size * 0.02, size * 0.035])
        ctx.strokeStyle = rgba(0.45)
        ctx.lineWidth = 1.5
        ctx.beginPath()
        ctx.arc(0, 0, radius + size * 0.025, 0, Math.PI * 2)
        ctx.stroke()
        ctx.restore()
        ctx.setLineDash([])

        // the orb: a blob whose surface wobbles with loudness
        const wobble = 0.25 + 1.6 * smoothLevel
        ctx.beginPath()
        const steps = 96
        for (let s = 0; s <= steps; s++) {
            const th = (s / steps) * Math.PI * 2
            const k =
                1 +
                wobble *
                    (0.05 * Math.sin(3 * th + t * 1.3) +
                        0.035 * Math.sin(5 * th - t * 2.1) +
                        0.025 * Math.sin(7 * th + t * 3.4))
            const x = c + Math.cos(th) * radius * k
            const y = c + Math.sin(th) * radius * k
            s === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y)
        }
        ctx.closePath()
        const body = ctx.createRadialGradient(c - radius * 0.3, c - radius * 0.35, radius * 0.05, c, c, radius * 1.1)
        body.addColorStop(0, "rgba(255, 255, 255, 0.95)")
        body.addColorStop(0.25, rgba(0.95))
        body.addColorStop(0.75, rgba(0.45))
        body.addColorStop(1, rgba(0.12))
        ctx.shadowColor = rgba(0.9)
        ctx.shadowBlur = size * (0.06 + 0.12 * smoothLevel)
        ctx.fillStyle = body
        ctx.fill()
        ctx.shadowBlur = 0
    }

    onMount(() => {
        frame = requestAnimationFrame(draw)
    })

    onDestroy(() => {
        cancelAnimationFrame(frame)
        unsubs.forEach((u) => u())
    })
</script>

<canvas
    bind:this={canvas}
    class="voice-orb"
    style="width: {size}px; height: {size}px"
    aria-label="Индикатор голоса"
></canvas>

<style>
    .voice-orb {
        display: block;
        margin: auto;
    }
</style>
