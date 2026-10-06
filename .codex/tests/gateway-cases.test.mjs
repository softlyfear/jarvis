import test from 'node:test';
import assert from 'node:assert/strict';
import { requestsForCase } from '../checks/gateway-cases.mjs';

const request = (label, tools = []) => ({messages: [{role: 'user', content: `CASE:${label}`}], tools});

test('late main-agent completion does not replace helper, vision, image or keyless requests', () => {
    // Reconstruct the observed ordering: planner request, then main spawn completion.
    const main = request('spawn', ['sessions_spawn']);
    for (const label of ['helper', 'vision', 'image', 'keyless']) {
        const target = request(label);
        for (const trace of [[target, main], [main, target], [main, target, main]]) {
            assert.deepEqual(requestsForCase(trace, label), [target]);
            assert.equal(requestsForCase(trace, label)[0], target);
        }
    }
});

test('every matching request survives selection, including an earlier policy violation', () => {
    const forbidden = request('helper', ['exec']);
    const clean = request('helper');
    const captured = [forbidden, request('spawn'), clean];
    assert.deepEqual(requestsForCase(captured, 'helper'), [forbidden, clean]);
    // Preserve object identity for the parallel authorization journal.
    assert.equal(captured.indexOf(requestsForCase(captured, 'helper')[0]), 0);
    assert.equal(captured.indexOf(requestsForCase(captured, 'helper')[1]), 2);
});

test('absent upstream request fails instead of vacuously passing an empty loop', () => {
    for (const label of ['helper', 'vision', 'image', 'keyless']) {
        assert.throws(() => requestsForCase([request('spawn')], label), /No upstream request/);
    }
});

test('assistant messages cannot impersonate a scenario; multimodal user messages match', () => {
    const unrelated = {messages: [{role: 'assistant', content: 'CASE:image'}]};
    assert.throws(() => requestsForCase([unrelated], 'image'), /No upstream request/);
    const image = {messages: [{role: 'user', content: [
        {type: 'text', text: 'CASE:image'}, {type: 'image_url', image_url: {url: 'data:image/png;base64,fixture'}}
    ]}]};
    assert.deepEqual(requestsForCase([unrelated, image, request('spawn')], 'image'), [image]);
});
