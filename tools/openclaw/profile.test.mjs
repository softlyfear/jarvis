import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { makeConfig, prepare } from './profile.mjs';
const providers = [{enabled: true, base_url: 'https://example.org/v1', keys: ['private-key'], models: ['test-model']}];
const temporaryRoot = path.join(process.cwd(), '.codex', '.tmp');
fs.mkdirSync(temporaryRoot, {recursive: true});

test('main agent owns PC tools; helpers cannot execute or delegate further', () => {
    const config = makeConfig(path.join(os.tmpdir(), 'jarvis-profile'), providers);
    assert.equal(config.gateway.bind, 'loopback');
    assert.equal(config.gateway.auth.token.length, 64);
    assert.equal(config.gateway.http.endpoints.chatCompletions.enabled, true);
    assert.deepEqual(config.agents.entries.jarvis.subagents.allowAgents, ['jarvis-research', 'jarvis-planner']);
    assert.equal(config.agents.defaults.subagents.maxSpawnDepth, 1);
    for (const [id, agent] of Object.entries(config.agents.entries).filter(([id]) => id !== 'jarvis-vision')) {
        assert.equal(agent.tools.fs.workspaceOnly, true);
        assert.ok(agent.tools.deny.includes('group:runtime'));
        if (id !== 'jarvis') { assert.ok(!agent.tools.allow.includes('bundle-mcp')); assert.ok(!agent.tools.allow.includes('sessions_spawn')); }
    }
    assert.deepEqual(config.agents.entries['jarvis-vision'].tools.deny, ['*']);
    assert.equal(config.tools.exec.security, 'deny');
    assert.ok(config.tools.subagents.tools.deny.includes('bundle-mcp'));
    assert.deepEqual(config.mcp.servers['jarvis-pc'].toolFilter.include, ['jarvis.*']);
});
test('updates preserve user memory, persona, MCP servers and Gateway identity', () => {
    const root = fs.mkdtempSync(path.join(temporaryRoot, 'jarvis-profile-'));
    try {
        const first = prepare(root, providers, 19876);
        const memory = path.join(root, 'workspace', 'MEMORY.md');
        const soul = path.join(root, 'workspace', 'SOUL.md');
        fs.writeFileSync(memory, 'Любимый цвет — зелёный.'); fs.writeFileSync(soul, 'Личная настройка.');
        first.mcp.servers.external = {url: 'https://example.org/mcp', enabled: false};
        fs.writeFileSync(path.join(root, 'openclaw.json'), JSON.stringify(first));
        const second = prepare(root, providers, 12345);
        assert.equal(second.gateway.port, 19876); assert.equal(second.gateway.auth.token, first.gateway.auth.token);
        assert.equal(second.mcp.servers.external.enabled, false);
        assert.equal(fs.readFileSync(memory, 'utf8'), 'Любимый цвет — зелёный.');
        assert.equal(fs.readFileSync(soul, 'utf8'), 'Личная настройка.');
        assert.ok(fs.existsSync(path.join(root, 'workspace', 'skills', 'jarvis-memory', 'SKILL.md')));
    } finally { fs.rmSync(root, {recursive: true, force: true}); }
});
test('unsafe provider URLs and missing credentials are rejected', () => {
    for (const base_url of ['http://example.org', 'https://key@example.org', 'https://example.org?token=secret']) {
        assert.throws(() => makeConfig('/tmp/jarvis-profile', [{...providers[0], base_url}]));
    }
    assert.throws(() => makeConfig('/tmp/jarvis-profile', []));
    const cfg = makeConfig('/tmp/jarvis-profile', [{...providers[0], keys: [], keyless: true}]);
    assert.equal(cfg.models.providers['jarvis-provider-0'].apiKey, 'keyless');
});
test('deleted managed keys and models are removed while custom configuration survives', () => {
    const root = path.join(os.tmpdir(), 'jarvis-profile');
    const first = makeConfig(root, [...providers, {...providers[0], keys: ['second-old-key']}]);
    first.models.providers.custom = {baseUrl: 'https://example.org/v1', apiKey: 'custom-key', models: [{id: 'custom-model'}]};
    first.agents.defaults.models['custom/custom-model'] = {};
    first.agents.entries.personal = {workspace: root};
    const next = makeConfig(root, [{...providers[0], keys: [], keyless: true}], first);
    assert.equal(next.models.mode, 'replace');
    assert.ok(!JSON.stringify(next).includes('second-old-key'));
    assert.ok(!next.models.providers['jarvis-provider-1']);
    assert.ok(!next.agents.defaults.models['jarvis-provider-1/test-model']);
    assert.equal(next.models.providers.custom.apiKey, 'custom-key');
    assert.ok(next.agents.entries.personal);
    assert.throws(() => makeConfig(root, [], first));
});
