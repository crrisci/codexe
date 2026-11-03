// Copyright (c) 2026 Francesco Pio Crisci
// SPDX-License-Identifier: Apache-2.0

// Acceptance through the rebuilt CLI, with a deterministic native service.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:net';
import { spawn } from 'node:child_process';
import { mkdir, mkdtemp, writeFile } from 'node:fs/promises';
import { once } from 'node:events';
import { isAbsolute, join, resolve } from 'node:path';

const binary = process.env.XENOCODEX_NATIVE_CODEX_BIN;
assert.ok(binary && isAbsolute(binary), 'Set XENOCODEX_NATIVE_CODEX_BIN to the absolute rebuilt fork binary');

async function invoke(t, { env: overrides = {}, config = {}, describe: capabilities = {}, stop = 'stop', describeAfterDiscovery } = {}) {
  await mkdir(resolve('.test-artifacts'), { recursive: true });
  const root = await mkdtemp(resolve('.test-artifacts/native-parity-'));
  const workspace = join(root, 'work'), home = join(root, 'home'), socketPath = join(root, 'w.sock');
  await Promise.all([mkdir(workspace), mkdir(home)]);
  const requests = [], failures = [], sockets = new Set();
  const describe = { ok: true, protocol: 1, model: 'discovered-parity-model', context_window: 98304,
    max_output: 12288, max_frame: 1048576, kvstore: true,
    reasoning: { efforts: ['low', 'medium', 'high', 'max'], history: ['discard', 'preserve_tool_calls'], budget_tokens: true },
    ...capabilities };
  const server = createServer(socket => {
    sockets.add(socket);
    socket.on('error', () => {});
    socket.on('close', () => sockets.delete(socket));
    let buffer = '';
    const send = value => socket.write(JSON.stringify(value) + '\n');
    socket.setEncoding('utf8').on('data', chunk => {
      buffer += chunk;
      for (;;) {
        const end = buffer.indexOf('\n');
        if (end < 0) break;
        const line = buffer.slice(0, end);
        buffer = buffer.slice(end + 1);
        try {
          const request = JSON.parse(line);
          requests.push(request);
          switch (request.op) {
            case 'describe': send(requests.filter(r => r.op === 'describe').length > 1 && describeAfterDiscovery ? { ...describe, ...describeAfterDiscovery } : describe); break;
            case 'create': send({ ok: true, session: 'cccccccccccccccccccccccccccccccc', marker: 0 }); break;
            case 'rebuild': send({ ok: true, marker: 0 }); break;
            case 'generate':
              assert.equal(requests.filter(r => r.op === 'generate').length, 1, 'Terminal completion must not retry inference');
              send({ event: 'start' });
              send({ event: 'text_delta', text: 'PARITY_TERMINAL' });
              send({ event: 'done', stop, marker: 1,
                usage: { input: 7, cache_read: 0, output: 5001, total: 5008, reasoning: 0, replayed: 0 },
                checkpoint: { saved: true, tokens: 5008 } });
              break;
            case 'checkpoint': send({ ok: true, saved: true, tokens: 5008 }); break;
            default: throw new Error(`Unexpected operation ${request.op}`);
          }
        } catch (error) {
          failures.push(error.stack);
          send({ event: 'error', code: 'invalid_request', error: error.message });
        }
      }
    });
  });
  server.listen(socketPath);
  await once(server, 'listening');
  t.after(async () => {
    for (const socket of sockets) socket.destroy();
    await new Promise(resolve => server.close(resolve));
  });
  const flags = Object.entries({ model_provider: 'xenolith', web_search: 'disabled',
    'model_providers.xenolith.name': 'Xenolith', 'model_providers.xenolith.base_url': `unix://${socketPath}`,
    'model_providers.xenolith.wire_api': 'xenolith', 'model_providers.xenolith.requires_openai_auth': false,
    'model_providers.xenolith.supports_websockets': false, 'model_providers.xenolith.stream_max_retries': 0,
    'model_providers.xenolith.request_max_retries': 0, ...config,
  }).flatMap(([key, value]) => ['-c', `${key}=${JSON.stringify(value)}`]);
  const env = { ...process.env };
  for (const key of ['XENOLITH_MAX_OUTPUT_TOKENS', 'XENOLITH_REASONING_BUDGET', 'XENOLITH_REASONING_HISTORY']) delete env[key];
  Object.assign(env, { CODEX_HOME: home, XENOLITH_STATE_DIR: join(root, 'bindings') }, overrides);
  const child = spawn(binary, ['--no-daemon', 'exec', '--ignore-user-config', '--ignore-rules', '--skip-git-repo-check',
    '--sandbox', 'read-only', '--json', '-C', workspace, ...flags, 'Return the native service response.'],
  { cwd: workspace, env, stdio: ['ignore', 'pipe', 'pipe'] });
  let stdout = '', stderr = '';
  child.stdout.setEncoding('utf8').on('data', chunk => { stdout += chunk; });
  child.stderr.setEncoding('utf8').on('data', chunk => { stderr += chunk; });
  const timer = setTimeout(() => child.kill('SIGKILL'), 45000);
  let code, signal;
  try {
    ({ code, signal } = await new Promise((resolve, reject) => {
      child.once('error', reject);
      child.once('close', (code, signal) => resolve({ code, signal }));
    }));
  } finally {
    clearTimeout(timer);
    await Promise.all([
      writeFile(join(root, 'stdout.jsonl'), stdout), writeFile(join(root, 'stderr.log'), stderr),
      writeFile(join(root, 'wire.json'), JSON.stringify(requests, null, 2)),
      writeFile(join(root, 'failures.json'), JSON.stringify(failures, null, 2)),
    ]);
  }
  assert.equal(signal, null, `CLI timed out or crashed: ${root}\n${stderr}`);
  assert.equal(failures.length, 0, failures.join('\n'));
  return { code, requests, stdout, stderr, root };
}

function succeeded(result) {
  assert.equal(result.code, 0, `${result.root}\n${result.stderr}\n${result.stdout}`);
  const events = result.stdout.trim().split('\n').map(line => JSON.parse(line));
  assert.equal(events.findLast(e => e.type === 'item.completed' && e.item?.type === 'agent_message')?.item.text, 'PARITY_TERMINAL');
  assert.ok(events.some(event => event.type === 'turn.completed'));
  const generation = result.requests.find(r => r.op === 'generate');
  assert.ok(generation);
  assert.ok(result.requests.filter(r => r.op === 'describe').length >= 2, 'Describe discovers config and validates inference');
  return generation;
}

test('omitted model, context and effort discover the service and use its full output limit', { timeout: 60000 }, async t => {
  const generation = succeeded(await invoke(t));
  assert.equal(generation.max_tokens, 12288);
  assert.equal(generation.reasoning, false, 'Unconfigured reasoning remains disabled');
});

test('a conservative manual context budget below discovered capacity is accepted', { timeout: 60000 }, async t => {
  succeeded(await invoke(t, { config: { model_context_window: 65536 } }));
});

test('default output follows a fresh service limit before inference', { timeout: 60000 }, async t => {
  const generation = succeeded(await invoke(t, { describeAfterDiscovery: { max_output: 6000 } }));
  assert.equal(generation.max_tokens, 6000);
});

test('output override above 4096 is honored and done length terminates the turn', { timeout: 60000 }, async t => {
  const generation = succeeded(await invoke(t, { env: { XENOLITH_MAX_OUTPUT_TOKENS: '9000' }, stop: 'length' }));
  assert.equal(generation.max_tokens, 9000);
});

for (const [effort, expected] of [['minimal', 'low'], ['xhigh', 'high']]) {
  test(`native effort ${effort} maps to service ${expected} with discard history`, { timeout: 60000 }, async t => {
    const generation = succeeded(await invoke(t, { config: { model_reasoning_effort: effort }, env: { XENOLITH_REASONING_BUDGET: '0' } }));
    assert.deepEqual(generation.reasoning, { effort: expected, budget_tokens: 0, history: 'discard' });
  });
}

const invalidCases = [
  ['zero output', { env: { XENOLITH_MAX_OUTPUT_TOKENS: '0' } }, /XENOLITH_MAX_OUTPUT_TOKENS/],
  ['noninteger output', { env: { XENOLITH_MAX_OUTPUT_TOKENS: '9.5' } }, /XENOLITH_MAX_OUTPUT_TOKENS/],
  ['output exceeding service', { env: { XENOLITH_MAX_OUTPUT_TOKENS: '12289' } }, /XENOLITH_MAX_OUTPUT_TOKENS|max.output|output.limit/i],
  ['negative reasoning budget', { env: { XENOLITH_REASONING_BUDGET: '-1' } }, /XENOLITH_REASONING_BUDGET/],
  ['overflowing reasoning budget', { env: { XENOLITH_REASONING_BUDGET: '2147483648' } }, /XENOLITH_REASONING_BUDGET/],
  ['unknown reasoning history', { env: { XENOLITH_REASONING_HISTORY: 'everything' } }, /XENOLITH_REASONING_HISTORY/],
  ['model conflicting with discovery', { config: { model: 'wrong-model' } }, /model/i],
  ['context exceeding discovery', { config: { model_context_window: 98305 } }, /context/i],
  ['context changed after discovery', { describeAfterDiscovery: { context_window: 65536 } }, /context/i],
  ['output override invalidated after discovery', { env: { XENOLITH_MAX_OUTPUT_TOKENS: '9000' }, describeAfterDiscovery: { max_output: 6000 } }, /output|max_tokens|limit/i],
  ['reasoning capabilities changed after discovery', { config: { model_reasoning_effort: 'high' }, describeAfterDiscovery: { reasoning: { efforts: ['low'], history: ['discard'], budget_tokens: true } } }, /reasoning|effort/i],
  ['model changed after discovery', { describeAfterDiscovery: { model: 'changed-model' } }, /model/i],
  ['zero context', { config: { model_context_window: 0 } }, /context/i],
  ['auto compaction above context', { config: { model_auto_compact_token_limit: 98305 } }, /compact|context/i],
  ['budget without reasoning', { env: { XENOLITH_REASONING_BUDGET: '5' } }, /reasoning|budget/i],
  ['unsupported reasoning effort', { config: { model_reasoning_effort: 'high' }, describe: { reasoning: { efforts: ['low'], history: ['discard'], budget_tokens: true } } }, /reasoning|effort/i],
  ['unsupported reasoning budget', { config: { model_reasoning_effort: 'low' }, env: { XENOLITH_REASONING_BUDGET: '5' }, describe: { reasoning: { efforts: ['low'], history: ['discard'], budget_tokens: false } } }, /reasoning|budget/i],
  ['unsupported reasoning history', { config: { model_reasoning_effort: 'low' }, env: { XENOLITH_REASONING_HISTORY: 'preserve_tool_calls' }, describe: { reasoning: { efforts: ['low'], history: ['discard'], budget_tokens: true } } }, /reasoning|history/i],
];
for (const [name, options, message] of invalidCases) {
  test(`reject ${name} before inference`, { timeout: 60000 }, async t => {
    const result = await invoke(t, options);
    assert.notEqual(result.code, 0, `${result.root}\n${result.stdout}`);
    assert.match(result.stderr + result.stdout, message);
    assert.equal(result.requests.filter(r => r.op === 'generate').length, 0, 'Invalid configuration must not start inference');
    assert.equal(result.requests.filter(r => ['create', 'open', 'rebuild', 'append'].includes(r.op)).length, 0, 'Invalid configuration must not mutate or bind a native session');
  });
}
