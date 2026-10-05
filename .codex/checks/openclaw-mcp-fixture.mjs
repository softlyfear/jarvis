import readline from 'node:readline';
for await (const line of readline.createInterface({input: process.stdin})) {
    const request = JSON.parse(line);
    if (!Object.hasOwn(request, 'id')) continue;
    let result;
    switch (request.method) {
        case 'initialize': result = {protocolVersion: '2025-11-25', capabilities: {tools: {}}, serverInfo: {name: 'jarvis-test', version: '1'}}; break;
        case 'tools/list': result = {tools: [
            {name: 'jarvis.test', description: 'Read-only integration test', inputSchema: {type: 'object', properties: {}}},
            {name: 'forbidden_tool', description: 'Must be filtered', inputSchema: {type: 'object', properties: {}}}
        ]}; break;
        case 'tools/call': result = {content: [{type: 'text', text: 'MCP_NATIVE_OK'}]}; break;
        default: result = {};
    }
    process.stdout.write(JSON.stringify({jsonrpc: '2.0', id: request.id, result}) + '\n');
}
