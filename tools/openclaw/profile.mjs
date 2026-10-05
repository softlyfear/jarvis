// Managed OpenClaw state is separate from the user's other installations.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

export const OPENCLAW_VERSION = '2026.9.8';
export const NODE_VERSION = '24.16.0';
const here = path.dirname(fileURLToPath(import.meta.url));
const denied = ['group:runtime', 'group:nodes', 'group:ui', 'group:automation', 'group:messaging', 'group:media'];
const common = ['read', 'write', 'edit', 'memory_search', 'memory_get', 'web_search', 'web_fetch', 'session_status'];

export function makeConfig(root, providers, previous = {}, port = 18790) {
    const catalog = {};
    const registered = {};
    for (const [index, p] of providers.entries()) {
        if (!p.enabled || (!p.keyless && !p.keys?.some(k => k.trim())) || !p.models?.length) continue;
        const id = `jarvis-provider-${index}`;
        const url = new URL(p.base_url);
        if (url.username || url.password || url.search || url.hash ||
            (url.protocol !== 'https:' && !(url.protocol === 'http:' && ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname)))) {
            throw new Error('Провайдер: нужен HTTPS или локальный HTTP без ключей в адресе.');
        }
        registered[id] = {
            baseUrl: p.base_url, api: 'openai-completions',
            // The OpenAI client requires a nonempty credential; Kilo explicitly
            // recognizes "anonymous" for free models. Ignore stale keys in this mode.
            apiKey: p.keyless ? 'anonymous' : p.keys.find(k => k.trim()),
            // Capability metadata is explicit for the two verified imported Gemini models.
            models: p.models.map(model => ({id: model, name: model,
                input: ['google/gemini-3.5-flash', 'google/gemini-3.5-flash-lite'].includes(model) ? ['text', 'image'] : ['text']}))
        };
        for (const model of p.models) catalog[`${id}/${model}`] = {};
    }
    const models = Object.keys(catalog);
    const previousModel = previous.agents?.defaults?.model;
    const previousPrimary = typeof previousModel === 'string' ? previousModel : previousModel?.primary;
    if (!models.length && (!previousPrimary || previousPrimary.startsWith('jarvis-provider-'))) throw new Error('Добавьте ключ модели или включите бесплатные модели перед настройкой.');
    const retainCustom = values => Object.fromEntries(Object.entries(values || {}).filter(([id]) => !id.startsWith('jarvis-provider-')));
    const model = models.length ? {primary: models[0], fallbacks: models.slice(1)} : previous.agents.defaults.model;
    const agent = (id, name, allow, subagents = undefined) => ({
        ...previous.agents?.entries?.[id],
        name, workspace: path.join(root, 'workspace'),
        ...(subagents ? {subagents} : {}), heartbeat: {every: '0m'},
        tools: {allow, deny: denied, fs: {workspaceOnly: true}}
    });
    return {
        ...previous,
        logging: {...previous.logging, file: path.join(root, 'gateway-detail.log')},
        plugins: {...previous.plugins, entries: {...previous.plugins?.entries,
            'memory-core': {...previous.plugins?.entries?.['memory-core'], config: {...previous.plugins?.entries?.['memory-core']?.config, dreaming: {enabled: false}}}}},
        gateway: {...previous.gateway, mode: 'local', bind: 'loopback', port: previous.gateway?.port || port,
            auth: {mode: 'token', token: previous.gateway?.auth?.token || crypto.randomBytes(32).toString('hex')},
            http: {...previous.gateway?.http, endpoints: {...previous.gateway?.http?.endpoints, chatCompletions: {enabled: true}}}},
        agents: {ownership: 'explicit', defaults: {...previous.agents?.defaults, model,
            models: {...retainCustom(previous.agents?.defaults?.models), ...catalog}, timeoutSeconds: 300,
            subagents: {maxConcurrent: 2, maxSpawnDepth: 1, runTimeoutSeconds: 120}},
            entries: {
                ...previous.agents?.entries,
                jarvis: agent('jarvis', 'Джарвис', [...common, 'sessions_spawn', 'subagents', 'bundle-mcp'],
                    {allowAgents: ['jarvis-research', 'jarvis-planner'], requireAgentId: true}),
                'jarvis-research': agent('jarvis-research', 'Исследователь', ['read', 'memory_search', 'memory_get', 'web_search', 'web_fetch', 'session_status']),
                'jarvis-planner': agent('jarvis-planner', 'Планировщик', ['read', 'memory_search', 'memory_get', 'session_status']),
                'jarvis-vision': {...agent('jarvis-vision', 'Зрение', []), tools: {deny: ['*'], fs: {workspaceOnly: true}}}}},
        models: {...previous.models, mode: 'replace', providers: {...retainCustom(previous.models?.providers), ...registered}},
        tools: {...previous.tools, codeMode: false, toolSearch: false, elevated: {enabled: false}, exec: {security: 'deny', ask: 'off'},
            fs: {workspaceOnly: true}, sessions: {visibility: 'tree'}, agentToAgent: {enabled: false},
            subagents: {tools: {deny: ['bundle-mcp', 'sessions_spawn', ...denied]}}},
        skills: {...previous.skills, allowBundled: []},
        mcp: {...previous.mcp, servers: {...previous.mcp?.servers,
            'jarvis-pc': {command: path.join(process.env.JARVIS_APP_DIR || path.resolve(here, '../..'), process.platform === 'win32' ? 'jarvis-pc.exe' : 'jarvis-pc'),
                transport: 'stdio', enabled: true, supportsParallelToolCalls: false,
                requestTimeoutMs: 120000, toolFilter: {include: ['jarvis.*']}}}}
    };
}

export function prepare(root, providers, port) {
    fs.mkdirSync(root, {recursive: true, mode: 0o700});
    const configPath = path.join(root, 'openclaw.json');
    const previous = fs.existsSync(configPath) ? JSON.parse(fs.readFileSync(configPath, 'utf8')) : {};
    const config = makeConfig(root, providers, previous, port);
    const workspace = path.join(root, 'workspace');
    fs.mkdirSync(path.join(workspace, 'memory'), {recursive: true});
    // Copy missing templates only: persona edits, notes and memory survive updates.
    fs.cpSync(path.join(here, 'workspace'), workspace, {recursive: true, force: false, errorOnExist: false});
    const temp = configPath + '.new';
    fs.writeFileSync(temp, JSON.stringify(config, null, 2), {mode: 0o600});
    fs.renameSync(temp, configPath);
    fs.chmodSync(configPath, 0o600);
    return config;
}

export function environment(root) {
    return {...process.env, OPENCLAW_STATE_DIR: root, OPENCLAW_CONFIG_PATH: path.join(root, 'openclaw.json'),
        OPENCLAW_NO_RESPAWN: '1', PATH: path.dirname(process.execPath) + path.delimiter + process.env.PATH};
}

async function main() {
    const root = process.env.JARVIS_OPENCLAW_DIR;
    if (!root || !path.isAbsolute(root)) throw new Error('Не указан каталог профиля OpenClaw.');
    const command = process.argv[2];
    if (command === 'prepare') {
        let input = '';
        for await (const chunk of process.stdin) { input += chunk; if (input.length > 1048576) throw new Error('Слишком большой профиль.'); }
        const {providers, port} = JSON.parse(input);
        const config = prepare(root, providers, port);
        // This private pipe, never a log or argv, returns the Gateway credential.
        process.stdout.write(JSON.stringify({base_url: `http://127.0.0.1:${config.gateway.port}`, api_key: config.gateway.auth.token, agent: 'jarvis'}));
    } else if (command === 'start') {
        const config = JSON.parse(fs.readFileSync(path.join(root, 'openclaw.json'), 'utf8'));
        const url = `http://127.0.0.1:${config.gateway.port}/v1/models`;
        let response;
        try { response = await fetch(url, {headers: {Authorization: `Bearer ${config.gateway.auth.token}`}, signal: AbortSignal.timeout(2000)}); } catch {}
        if (response) {
            if (response.ok && (await response.json()).data?.some(x => x.id === 'openclaw/jarvis')) return;
            throw new Error('Порт Gateway занят другой программой.');
        }
        const entry = path.join(root, 'runtime', 'node_modules', 'openclaw', 'openclaw.mjs');
        const log = fs.openSync(path.join(root, 'gateway.log'), 'a', 0o600);
        const child = spawn(process.execPath, [entry, 'gateway', 'run'], {env: environment(root), detached: true, windowsHide: true, stdio: ['ignore', log, log]});
        child.unref(); fs.closeSync(log);
        for (let i = 0; i < 60; i++) {
            try {
                const ready = await fetch(url, {headers: {Authorization: `Bearer ${config.gateway.auth.token}`}, signal: AbortSignal.timeout(1000)});
                if (ready.ok && (await ready.json()).data?.some(x => x.id === 'openclaw/jarvis')) return;
            } catch {}
            await new Promise(resolve => setTimeout(resolve, 500));
        }
        throw new Error('Gateway не запустился.');
    } else if (command === 'models') {
        const config = JSON.parse(fs.readFileSync(path.join(root, 'openclaw.json'), 'utf8'));
        const models = new Set(Object.keys(config.agents?.defaults?.models || {}));
        for (const [id, p] of Object.entries(config.models?.providers || {})) for (const m of p.models || []) models.add(`${id}/${m.id}`);
        process.stdout.write(JSON.stringify([...models]));
    } else if (command === 'dashboard') {
        const entry = path.join(root, 'runtime', 'node_modules', 'openclaw', 'openclaw.mjs');
        const child = spawn(process.execPath, [entry, 'dashboard', '--json'], {env: environment(root), windowsHide: true, stdio: ['ignore', 'pipe', 'ignore']});
        let output = '';
        for await (const chunk of child.stdout) { output += chunk; if (output.length > 1048576) { child.kill(); throw new Error('Слишком большой ответ Gateway.'); } }
        const handoff = JSON.parse(output);
        if (!handoff.browserUrl) throw new Error('Не удалось открыть настройки Gateway.');
        process.stdout.write(JSON.stringify({browserUrl: handoff.browserUrl}));
    } else throw new Error('Неизвестная операция настройки.');
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
    main().catch(() => { process.stderr.write('Не удалось настроить или запустить OpenClaw. Проверьте gateway.log и настройки профиля.\n'); process.exitCode = 1; });
}
