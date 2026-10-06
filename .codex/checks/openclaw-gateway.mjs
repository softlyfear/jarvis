// Real pinned Gateway with a deterministic local provider; no user credentials.
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { prepare, environment } from '../../tools/openclaw/profile.mjs';
import { requestsForCase } from './gateway-cases.mjs';
const project = process.cwd();
const root = fs.mkdtempSync(path.join(project, '.codex/.tmp/real-gateway-'));
const entry = path.join(project, '.codex/.tmp/openclaw-runtime/node_modules/openclaw/openclaw.mjs');
let captured = [];
const providerAuth = [];
let cancelStarted;
let cancelClosed;
const upstreamStarted = new Promise(resolve => { cancelStarted = resolve; });
const upstreamClosed = new Promise(resolve => { cancelClosed = resolve; });
const provider = http.createServer(async (req, res) => {
    let body = ''; for await (const chunk of req) body += chunk;
    const data = JSON.parse(body); captured.push(data); providerAuth.push(req.headers.authorization);
    const messages = data.messages || [];
    const user = messages.filter(m => m.role === 'user').map(m => typeof m.content === 'string' ? m.content : JSON.stringify(m.content)).join('\n');
    const toolReplies = messages.filter(m => m.role === 'tool');
    const tools = (data.tools || []).map(t => t.function.name);
    let text = 'Настоящий Gateway ответил по-русски.';
    let call;
    if (user.includes('CASE:cancel')) {
        res.writeHead(200, {'content-type': 'text/event-stream'});
        res.write('data: {"choices":[{"index":0,"delta":{"role":"assistant","content":"Жду."}}]}\n\n');
        res.on('close', cancelClosed); cancelStarted(); return;
    }
    if (user.includes('CASE:memory') && !toolReplies.length) call = {name: 'write', arguments: {path: 'memory/gateway-test.md', content: 'Любимый цвет владельца — малахитовый.\n'}};
    if (user.includes('CASE:outside') && !toolReplies.length) call = {name: 'write', arguments: {path: path.join(root, 'forbidden.txt'), content: 'must not exist'}};
    if (user.includes('CASE:search') && !toolReplies.length) call = {name: 'memory_search', arguments: {query: 'малахитовый'}};
    if (user.includes('CASE:spawn') && !toolReplies.length) call = {name: 'sessions_spawn', arguments: {agentId: 'jarvis-planner', task: 'Проверь, что ты не можешь выполнять команды Windows.', mode: 'run'}};
    if (user.includes('CASE:client')) {
        // HTTP client results are rendered into the agent's user transcript, while the
        // delegated internal tool result still carries its earlier "pending" marker.
        if (user.includes('Таймеров нет.')) text = 'Результат инструмента: Таймеров нет.';
        else call = {name: tools.includes('jarvis_client__timers') ? 'jarvis_client__timers' : 'timers', arguments: {action: 'left'}};
    }
    if (user.includes('CASE:mcp') && !toolReplies.length) {
        const name = tools.find(t => t.startsWith('jarvis-pc__') && t.includes('test'));
        assert.ok(name, 'MCP tool not discovered'); assert.ok(!tools.some(t => t.includes('forbidden_tool')));
        call = {name, arguments: {}};
    }
    if (toolReplies.length && !user.includes('CASE:client')) text = 'Результат инструмента: ' + JSON.stringify(toolReplies.at(-1).content);
    if (call) assert.ok(tools.includes(call.name), `${call.name} missing: ${tools.join(',')}`);
    res.writeHead(200, {'content-type': 'text/event-stream'});
    const delta = call ? {role: 'assistant', tool_calls: [{index: 0, id: `call-${captured.length}`, type: 'function', function: {name: call.name, arguments: JSON.stringify(call.arguments)}}]} : {role: 'assistant', content: text};
    res.write(`data: ${JSON.stringify({id: 'test', object: 'chat.completion.chunk', model: 'test-model', choices: [{index: 0, delta, finish_reason: null}]})}\n\n`);
    res.write(`data: ${JSON.stringify({id: 'test', object: 'chat.completion.chunk', model: 'test-model', choices: [{index: 0, delta: {}, finish_reason: call ? 'tool_calls' : 'stop'}], usage: {prompt_tokens: 20, completion_tokens: 20, total_tokens: 40}})}\n\n`);
    res.end('data: [DONE]\n\n');
});
await new Promise(resolve => provider.listen(0, '127.0.0.1', resolve));
const portProbe = http.createServer(); await new Promise(resolve => portProbe.listen(0, '127.0.0.1', resolve));
const port = portProbe.address().port; await new Promise(resolve => portProbe.close(resolve));
const providerUrl = `http://127.0.0.1:${provider.address().port}/v1`;
const config = prepare(root, [
    {enabled: true, base_url: providerUrl, models: ['test-model'], keys: ['local-test']},
    {enabled: true, base_url: providerUrl, models: ['anonymous-model'], keys: ['stale-key'], keyless: true}
], port);
// PC actions are tested in Rust; this harness runs filesystem, memory and delegation natively.
config.mcp.servers['jarvis-pc'].command = process.execPath;
config.mcp.servers['jarvis-pc'].args = [path.join(project, '.codex/checks/openclaw-mcp-fixture.mjs')];
config.models.providers['jarvis-provider-0'].models[0].input = ['text', 'image'];
fs.writeFileSync(path.join(root, 'openclaw.json'), JSON.stringify(config));
const log = fs.openSync(path.join(root, 'gateway.log'), 'a');
const gatewayEnvironment = environment(root);
// A test must never discover a developer's real embedding/provider credentials.
for (const key of Object.keys(gatewayEnvironment)) if (/(API.?KEY|TOKEN|SECRET|PASSWORD|CREDENTIAL)/i.test(key)) delete gatewayEnvironment[key];
gatewayEnvironment.NO_PROXY = '127.0.0.1,localhost,::1';
const child = spawn(process.execPath, [entry, 'gateway', 'run'], {env: gatewayEnvironment, stdio: ['ignore', log, log]});
fs.closeSync(log);
const base = `http://127.0.0.1:${port}`;
const headers = {Authorization: `Bearer ${config.gateway.auth.token}`, 'Content-Type': 'application/json'};
async function ask(label, stream = false, agent = 'jarvis') {
    const response = await fetch(base + '/v1/chat/completions', {method: 'POST', headers,
        body: JSON.stringify({model: `openclaw/${agent}`, user: `test-${label}`, stream, messages: [{role: 'user', content: `CASE:${label}`}]}), signal: AbortSignal.timeout(60000)});
    const text = await response.text(); assert.equal(response.status, 200, text); return text;
}
try {
    let ready = false;
    for (let i = 0; i < 120; i++) {
        if (child.exitCode !== null) throw new Error(fs.readFileSync(path.join(root, 'gateway.log'), 'utf8'));
        try { const response = await fetch(base + '/v1/models', {headers, signal: AbortSignal.timeout(1000)}); if (response.ok) { ready = true; break; } } catch {}
        await new Promise(resolve => setTimeout(resolve, 500));
    }
    assert.ok(ready, 'Gateway startup timeout');
    assert.equal((await fetch(base + '/v1/models')).status, 401);
    const inventory = await (await fetch(base + '/v1/models', {headers})).json();
    assert.ok(inventory.data.some(m => m.id === 'openclaw/jarvis'));
    assert.ok((await ask('hello')).includes('Настоящий Gateway'));
    const stream = await ask('stream', true); assert.ok(stream.includes('data:')); assert.ok(stream.includes('[DONE]'));
    await ask('memory'); assert.ok(fs.readFileSync(path.join(root, 'workspace/memory/gateway-test.md'), 'utf8').includes('малахитовый'));
    const outside = await ask('outside'); assert.ok(!fs.existsSync(path.join(root, 'forbidden.txt'))); assert.ok(/outside|workspace|denied|boundary/i.test(outside), outside);
    await new Promise(resolve => setTimeout(resolve, 2500));
    const search = await ask('search'); assert.ok(search.includes('малахитовый'), search);
    assert.ok(captured.some(x => JSON.stringify(x).includes('jarvis-memory')), 'workspace skill not loaded');
    const spawnReply = await ask('spawn'); assert.ok(/accepted|childSessionKey/i.test(spawnReply), spawnReply);
    assert.ok((await ask('mcp')).includes('MCP_NATIVE_OK'));
    await ask('helper', false, 'jarvis-planner');
    for (const request of requestsForCase(captured, 'helper')) {
        const helperTools = (request.tools || []).map(t => t.function.name);
        assert.ok(!helperTools.includes('exec') && !helperTools.includes('sessions_spawn'));
        assert.ok(!helperTools.some(x => x.startsWith('jarvis-pc__')));
    }
    await ask('vision', false, 'jarvis-vision');
    for (const request of requestsForCase(captured, 'vision')) assert.deepEqual(request.tools || [], []);
    const icon = fs.readFileSync(path.join(project, 'resources/icons/32x32.png')).toString('base64');
    const imageReply = await fetch(base + '/v1/chat/completions', {method: 'POST', headers: {...headers, 'x-openclaw-model': 'jarvis-provider-0/test-model'},
        body: JSON.stringify({model: 'openclaw/jarvis-vision', user: 'test-image', tools: [], tool_choice: 'none', messages: [{role: 'user', content: [
            {type: 'text', text: 'CASE:image'}, {type: 'image_url', image_url: {url: `data:image/png;base64,${icon}`}}]}]})});
    assert.equal(imageReply.status, 200, await imageReply.text());
    for (const request of requestsForCase(captured, 'image')) {
        assert.ok(request.messages.some(m => Array.isArray(m.content) && m.content.some(p => p.type === 'image_url')), 'image did not reach upstream');
        assert.deepEqual(request.tools || [], []);
    }
    const anonymousReply = await fetch(base + '/v1/chat/completions', {method: 'POST', headers: {...headers, 'x-openclaw-model': 'jarvis-provider-1/anonymous-model'},
        body: JSON.stringify({model: 'openclaw/jarvis', user: 'test-keyless', messages: [{role: 'user', content: 'CASE:keyless'}]})});
    assert.equal(anonymousReply.status, 200, await anonymousReply.text());
    for (const request of requestsForCase(captured, 'keyless')) {
        assert.equal(providerAuth[captured.indexOf(request)], 'Bearer anonymous', 'keyless provider sent an invalid or stale credential');
    }
    if (process.env.JARVIS_GATEWAY_TEST_RUN_RUST === '1') {
        const cargo = spawn('cargo', ['test', '-p', 'jarvis-core', '--no-default-features', '--features', 'reqwest,lua,ipc,nnnoiseless', '--lib', 'real_gateway_client_tool_contract', '--', '--ignored', '--nocapture'],
            {env: {...process.env, DOCS_RS: '1', JARVIS_GATEWAY_TEST_PROFILE: path.join(root, 'openclaw.json')}, stdio: 'inherit'});
        const code = await new Promise(resolve => cargo.once('exit', resolve)); assert.equal(code, 0);
    } else {
        const tools = [{type: 'function', function: {name: 'timers', description: 'Read timer status', parameters: {type: 'object', properties: {action: {type: 'string'}}}}}];
        const message = {role: 'user', content: 'CASE:client'};
        const first = await (await fetch(base + '/v1/chat/completions', {method: 'POST', headers,
            body: JSON.stringify({model: 'openclaw/jarvis', user: 'test-client', tools, messages: [message]})})).json();
        const assistant = first.choices[0].message; assert.equal(assistant.tool_calls[0].function.name, 'timers');
        const second = await (await fetch(base + '/v1/chat/completions', {method: 'POST', headers,
            body: JSON.stringify({model: 'openclaw/jarvis', user: 'test-client', tools, messages: [message, assistant,
                {role: 'tool', tool_call_id: assistant.tool_calls[0].id, content: 'Таймеров нет.'}]})})).json();
        assert.ok(second.choices[0].message.content.includes('Таймеров нет.'));
    }
    console.log('PASS Gateway actions; checking browser handoff and cancellation');
    // Keep the mock provider responsive while the CLI probes the Gateway. Windows
    // CLI startup is slower than Linux; bound it without blocking this event loop.
    const dashboard = spawn(process.execPath, [entry, 'dashboard', '--json'], {env: gatewayEnvironment, signal: AbortSignal.timeout(60000), stdio: ['ignore', 'pipe', 'pipe']});
    let dashboardOutput = '', dashboardError = '';
    dashboard.stdout.on('data', chunk => { dashboardOutput += chunk; });
    dashboard.stderr.on('data', chunk => { dashboardError += chunk; });
    const dashboardCode = await new Promise((resolve, reject) => { dashboard.once('error', reject); dashboard.once('close', resolve); });
    assert.equal(dashboardCode, 0, dashboardError);
    const handoff = JSON.parse(dashboardOutput); assert.ok(handoff.browserUrl.startsWith(base));
    assert.ok(!handoff.browserUrl.includes(config.gateway.auth.token), 'shared credential leaked into browser handoff');
    const controller = new AbortController();
    const cancelled = fetch(base + '/v1/chat/completions', {method: 'POST', headers, signal: controller.signal,
        body: JSON.stringify({model: 'openclaw/jarvis', user: 'test-cancel', stream: true, messages: [{role: 'user', content: 'CASE:cancel'}]})})
        .then(r => r.text()).catch(() => {});
    await Promise.race([upstreamStarted, new Promise((_, reject) => setTimeout(() => reject(new Error('cancel request did not start')), 10000).unref())]);
    controller.abort(); await cancelled;
    await Promise.race([upstreamClosed, new Promise((_, reject) => setTimeout(() => reject(new Error('Gateway did not disconnect upstream on cancellation')), 3000).unref())]);
    assert.equal(captured.filter(x => JSON.stringify(x.messages).includes('CASE:cancel')).length, 1);
    console.log('PASS real Gateway: auth, routing, SSE, workspace skill, memory write/search, filesystem boundary, helper delegation/policy, MCP discovery/filter/call, browser handoff, cancellation');
    console.log(`Evidence: ${root}`);
} finally {
    fs.writeFileSync(path.join(root, 'provider-requests.json'), JSON.stringify(captured, null, 2));
    child.kill('SIGTERM'); provider.closeAllConnections(); await new Promise(resolve => provider.close(resolve));
    await new Promise(resolve => { if (child.exitCode !== null) resolve(); else { child.once('exit', resolve); setTimeout(() => { child.kill('SIGKILL'); resolve(); }, 5000).unref(); } });
}
