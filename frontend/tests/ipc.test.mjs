import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'
import vm from 'node:vm'
import ts from 'typescript'

function loadIpc() {
    const sockets = [], timers = new Map()
    let nextTimer = 1
    class WebSocket {
        static CONNECTING = 0
        static OPEN = 1
        constructor(url) { this.url = url; this.readyState = 0; sockets.push(this) }
        open() { this.readyState = 1; this.onopen?.() }
        close() { this.readyState = 3; this.onclose?.() }
        send(data) { this.sent = data }
        message(data) { this.onmessage?.({ data: JSON.stringify(data) }) }
    }
    const writable = value => ({
        value,
        set(value) { this.value = value },
    })
    const source = readFileSync(new URL('../src/lib/ipc.ts', import.meta.url), 'utf8')
    const compiled = ts.transpileModule(source, {
        compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
    }).outputText
    const context = {
        exports: {}, WebSocket, console: { log() {}, error() {} },
        setTimeout(fn) { const id = nextTimer++; timers.set(id, fn); return id },
        clearTimeout(id) { timers.delete(id) },
        require(name) {
            if (name === 'svelte/store') return { writable }
            if (name === '@tauri-apps/api/window') return { getCurrentWindow() {} }
            throw new Error(`Unexpected dependency: ${name}`)
        },
    }
    vm.runInNewContext(compiled, context)
    return { api: context.exports, sockets, timers, tick() {
        const pending = [...timers.values()]; timers.clear(); pending.forEach(fn => fn())
    } }
}

test('a dropped connection resets the visualizer and reconnects once', () => {
    const { api, sockets, timers, tick } = loadIpc()
    api.enableIpc(); api.enableIpc()
    assert.equal(sockets.length, 1, 'do not duplicate a pending connection')
    sockets[0].open()
    sockets[0].message({ event: 'audio_level', level: 0.8, bands: [0.4] })
    sockets[0].message({ event: 'speaking', active: true })
    sockets[0].close()
    assert.equal(api.jarvisState.value, 'disconnected')
    assert.equal(api.audioLevel.value, 0)
    assert.equal(api.speaking.value, false)
    assert.equal(timers.size, 1)
    tick()
    assert.equal(sockets.length, 2)
    sockets[1].open()
    assert.equal(api.ipcConnected.value, true)
})

test('disabling cancels retries and ignores late events from the old socket', () => {
    const { api, sockets, timers, tick } = loadIpc()
    api.enableIpc(); sockets[0].open(); sockets[0].close()
    api.disableIpc(); tick()
    assert.equal(sockets.length, 1)
    assert.equal(timers.size, 0)
    api.enableIpc(); sockets[1].open()
    sockets[0].onopen(); sockets[0].onclose()
    sockets[0].message({ event: 'speaking', active: true })
    assert.equal(api.ipcConnected.value, true)
    assert.equal(api.speaking.value, false)
    assert.equal(timers.size, 0)
})
