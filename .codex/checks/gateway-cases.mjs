import assert from 'node:assert/strict';

// Concurrent helper completions may arrive after a different agent's request.
export function requestsForCase(captured, label) {
    const requests = captured.filter(data => (data.messages || []).some(m =>
        m.role === 'user' && (typeof m.content === 'string' ? m.content : JSON.stringify(m.content)).includes(`CASE:${label}`)));
    assert.ok(requests.length, `No upstream request for ${label}`);
    return requests;
}
