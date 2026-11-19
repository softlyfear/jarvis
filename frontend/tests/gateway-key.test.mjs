import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'
import vm from 'node:vm'
import ts from 'typescript'

function load() {
    const source = readFileSync(new URL('../src/lib/gateway-key.ts', import.meta.url), 'utf8')
    const compiled = ts.transpileModule(source, {
        compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
    }).outputText
    const context = { exports: {} }
    vm.runInNewContext(compiled, context)
    return context.exports
}

const { assignGatewayKey } = load()
const both = {
    gateway: 'openrouter',
    openrouterKey: 'sk-or-live-openrouter-key',
    polzaKey: 'sk-polza-live-key-value',
}

test('editing an OpenRouter key cannot replace the Polza key', () => {
    const edited = assignGatewayKey(both, 'sk-v1-partial-edit-of-openrouter', false)
    assert.equal(edited.gateway, 'openrouter')
    assert.equal(edited.openrouterKey, 'sk-v1-partial-edit-of-openrouter')
    assert.equal(edited.polzaKey, both.polzaKey)
})

test('a pasted key selects its gateway without erasing the other one', () => {
    const polza = assignGatewayKey(both, 'sk-polza-new-key-from-paste', true)
    assert.equal(polza.gateway, 'polza')
    assert.equal(polza.polzaKey, 'sk-polza-new-key-from-paste')
    assert.equal(polza.openrouterKey, both.openrouterKey)

    const openrouter = assignGatewayKey({ ...both, gateway: 'polza' }, 'sk-or-new-openrouter-key', true)
    assert.equal(openrouter.gateway, 'openrouter')
    assert.equal(openrouter.openrouterKey, 'sk-or-new-openrouter-key')
    assert.equal(openrouter.polzaKey, both.polzaKey)
})

test('a short or non-key paste stays in the current field', () => {
    const short = assignGatewayKey(both, 'sk-short', true)
    assert.equal(short.gateway, 'openrouter')
    assert.equal(short.openrouterKey, 'sk-short')
    assert.equal(short.polzaKey, both.polzaKey)
})
