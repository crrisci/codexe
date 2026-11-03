// Copyright (c) 2026 Francesco Pio Crisci
// SPDX-License-Identifier: Apache-2.0

// Native fork acceptance test. Requires the rebuilt fork, never the installed CLI.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:net';
import { spawn } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { mkdir, mkdtemp, writeFile, readdir, readFile } from 'node:fs/promises';
import { resolve, join, isAbsolute } from 'node:path';
import { once } from 'node:events';

const binary = process.env.XENOCODEX_NATIVE_CODEX_BIN;
assert.ok(binary && isAbsolute(binary), 'Set XENOCODEX_NATIVE_CODEX_BIN to the absolute rebuilt fork binary');

for (const vision of [false, true]) for (const compact of [false, true]) test(`discovered native ${vision ? 'vision' : 'text'} model streams reasoning, executes a tool and resumes ${compact ? 'compacted' : 'persistent'} history`, { timeout: 120000 }, async t => {
  const parent = resolve('.test-artifacts');
  await mkdir(parent, { recursive: true });
  const root = await mkdtemp(join(parent, 'native-'));
  const workspace = join(root, 'workspace');
  const home = join(root, 'codex-home');
  const bindings = join(root, 'bindings');
  await Promise.all([mkdir(workspace), mkdir(home)]);
  const nonce = `NATIVE_${randomBytes(16).toString('hex')}`;
  const thought = `THOUGHT_${randomBytes(16).toString('hex')}`;
  const imageBudget = compact ? 70 : 140;
  await writeFile(join(workspace, 'input.txt'), nonce + '\n');
  const imagePath = join(workspace, 'image.png');
  if (vision) await writeFile(imagePath, Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC', 'base64'));
  const socketPath = join(root, 'w.sock');
  const sockets = new Set();
  const requests = [];
  const sessions = new Map();
  const failures = [];
  let generations = 0;
  const server = createServer(socket => {
    sockets.add(socket);
    socket.on('close', () => sockets.delete(socket));
    socket.on('error', () => {});
    let buffer = '', session;
    const send = value => socket.write(JSON.stringify(value) + '\n');
    socket.setEncoding('utf8').on('data', chunk => {
      buffer += chunk;
      for (;;) {
        const end = buffer.indexOf('\n');
        if (end < 0) break;
        const line = buffer.slice(0, end);
        buffer = buffer.slice(end + 1);
        try {
          const r = JSON.parse(line);
          requests.push(r);
          if (r.op === 'describe') send({ ok: true, protocol: vision ? 2 : 1, model: 'xenocodex-sim', context_window: 65536,
            max_output: 8192, max_frame: 1048576, kvstore: true,
            ...(vision ? { vision: { formats: ['png', 'jpeg'], budgets: [70, 140, 280, 560, 1120], default_budget: 280, max_bytes: 48 * 1024 * 1024, max_context_media_bytes: 256 * 1024 * 1024, media_refs: true } } : {}),
            reasoning: { efforts: ['low', 'medium', 'high', 'max'], history: ['discard', 'preserve_tool_calls'], budget_tokens: true } });
          else if (r.op === 'create') {
            session = randomBytes(16).toString('hex');
            sessions.set(session, { marker: 0, pending: [], tools: r.tools });
            send({ ok: true, session, marker: 0 });
          } else if (r.op === 'open') {
            session = r.session;
            const state = sessions.get(session);
            assert.ok(state, 'Reopen only a session created by this test');
            send({ ok: true, tokens: 600, marker: state.marker, pending: state.pending,
              turn_open: false, zero_prefill: true, resume: 'snapshot' });
          } else if (r.op === 'rebuild') {
            assert.ok(sessions.has(session));
            sessions.get(session).tools = r.tools;
            sessions.get(session).pending = [];
            if (vision && generations === 0) {
              assert.ok(r.messages.some(message => message.image?.data), 'CLI attachment must reach the engine as actual bytes');
              assert.ok(r.messages.filter(message => message.image).every(message => message.image.budget === imageBudget), 'Budget follows config.toml and its environment override');
            }
            if (compact && generations === 1) {
              assert.equal(r.tools.length, 0, 'Local compaction must not expose executable tools');
              assert.ok(JSON.stringify(r.messages).includes('PARITY_COMPACT_REQUEST'));
              if (vision) assert.ok(r.messages.some(message => message.role === 'tool' && message.image?.data), 'Compaction receives the actual tool image');
              else assert.ok(JSON.stringify(r.messages).includes(nonce), 'Compaction receives the actual executed tool result');
            }
            if (compact && generations === 2) {
              assert.ok(JSON.stringify(r.messages).includes('PARITY_SUMMARY_'), 'Summary replacement rebuilds the native history');
              assert.ok(!r.messages.some(message => message.role === 'tool'), 'Prior tool effects are represented by the summary');
            }
            send({ ok: true, marker: 0 });
          } else if (r.op === 'append') {
            if (r.role === 'tool') {
              assert.equal(r.call_id, 17);
              if (vision) {
                assert.ok(r.image?.data, 'Actual view_image must return image bytes');
                assert.equal(r.image.budget, imageBudget);
                assert.ok(Buffer.from(r.image.data, 'base64').length > 8);
              } else assert.ok(r.text.includes(nonce), 'Result must come from the actual executed tool');
              sessions.get(session).pending = [];
            } else assert.equal(r.role, 'user');
            send({ ok: true, marker: sessions.get(session).marker });
          } else if (r.op === 'rewind_cost') {
            send({ ok: true, prefill: 50 });
          } else if (r.op === 'generate') {
            const state = sessions.get(session);
            assert.ok(state);
            assert.deepEqual(r.reasoning, { effort: 'high', budget_tokens: 512, history: 'preserve_tool_calls' });
            assert.equal(r.max_tokens, 8192, 'Default output must use the advertised service limit, above 4096');
            generations++;
            assert.ok(generations <= (compact ? 4 : 3), 'No unexpected inference retries');
            send({ event: 'start' });
            send({ event: 'reasoning_delta', text: thought.slice(0, 10) });
            send({ event: 'reasoning_delta', text: thought.slice(10) });
            send({ event: 'inference_progress', phase: 'prefill', state: 'running', tokens: 0, total: 10, elapsed_ms: 0 });
            send({ event: 'inference_progress', phase: 'prefill', state: 'finished', tokens: 10, total: 10, elapsed_ms: 5 });
            if (generations === 1) {
              const name = vision ? 'view_image' : 'exec_command';
              assert.ok(state.tools.some(tool => tool.name === name), `Actual Codex ${name} declaration required`);
              send({ event: 'toolcall_start', id: 17, name });
              send({ event: 'toolcall_end', id: 17, name, arguments: vision ? { path: imagePath } : { cmd: 'cat input.txt', login: false, max_output_tokens: 1000, yield_time_ms: 1000 } });
              state.pending = [17];
            } else {
              const response = compact && generations === 2 ? `PARITY_SUMMARY_${nonce}` : generations === (compact ? 3 : 2) ? nonce : 'NATIVE_REOPENED';
              send({ event: 'text_delta', text: response });
            }
            state.marker++;
            send({ event: 'done', stop: generations === 1 ? 'tool_use' : 'stop', marker: state.marker,
              usage: { input: compact && generations === 1 ? 59000 : 6, cache_read: 600, output: 3, total: compact && generations === 1 ? 59603 : 609, reasoning: 2, replayed: 0 },
              checkpoint: { saved: true, tokens: 609 } });
          } else throw new Error(`Unexpected native operation ${r.op}`);
        } catch (error) {
          failures.push(error.stack);
          send({ event: 'error', code: 'invalid_request', error: error.message });
        }
      }
    });
  });
  server.listen(socketPath);
  await once(server, 'listening');
  t.after(async () => { for (const socket of sockets) socket.destroy(); await new Promise(resolve => server.close(resolve)); });
  const config = {
    model_provider: 'xenolith', model_reasoning_effort: 'high', web_search: 'disabled',
    'model_providers.xenolith.name': 'Xenolith', 'model_providers.xenolith.base_url': `unix://${socketPath}`,
    'model_providers.xenolith.wire_api': 'xenolith', 'model_providers.xenolith.requires_openai_auth': false,
    'model_providers.xenolith.supports_websockets': false, 'model_providers.xenolith.stream_max_retries': 0,
    'model_providers.xenolith.request_max_retries': 0,
  };
  if (compact) Object.assign(config, { compact_prompt: 'PARITY_COMPACT_REQUEST', model_auto_compact_token_limit: 40000, model_post_turn_compact_threshold_percent: 0 });
  const flags = Object.entries(config).flatMap(([key, value]) => ['-c', `${key}=${JSON.stringify(value)}`]);
  if (vision) await writeFile(join(home, 'config.toml'), '[model_providers.xenolith]\nimage_budget = 140\n');
  async function invoke(resume, instruction) {
    const args = ['--no-daemon', 'exec', ...(vision ? [] : ['--ignore-user-config']), '--ignore-rules',
      '--skip-git-repo-check', '--sandbox', 'read-only', '--json', '-C', workspace,
      ...flags, ...(resume ? ['resume', '--last'] : vision ? ['--image', imagePath] : []), '--', instruction];
    const inheritedEnv = { ...process.env };
    delete inheritedEnv.XENOLITH_MAX_OUTPUT_TOKENS;
    delete inheritedEnv.XENOLITH_IMAGE_BUDGET;
    if (vision && compact) inheritedEnv.XENOLITH_IMAGE_BUDGET = String(imageBudget);
    const child = spawn(binary, args, { cwd: workspace, env: { ...inheritedEnv, CODEX_HOME: home, XENOLITH_STATE_DIR: bindings, XENOLITH_REASONING_BUDGET: '512', XENOLITH_REASONING_HISTORY: 'preserve_tool_calls' }, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '', stderr = '';
    child.stdout.setEncoding('utf8').on('data', chunk => { stdout += chunk; });
    child.stderr.setEncoding('utf8').on('data', chunk => { stderr += chunk; });
    const timer = setTimeout(() => child.kill('SIGTERM'), 45000);
    let code;
    try { code = await new Promise((resolve, reject) => { child.once('error', reject); child.once('close', resolve); }); }
    finally { clearTimeout(timer); }
    await writeFile(join(root, resume ? 'resume.stdout.jsonl' : 'first.stdout.jsonl'), stdout);
    await writeFile(join(root, resume ? 'resume.stderr.log' : 'first.stderr.log'), stderr);
    assert.equal(code, 0, stderr + stdout + failures.join('\n'));
    return stdout.split('\n').filter(Boolean).map(line => JSON.parse(line));
  }
  try {
    const first = await invoke(false, vision ? 'Inspect the attached image and use view_image to open image.png.' : 'Read input.txt using exec_command and report its exact contents.');
    if (!vision) assert.ok(first.some(e => e.type === 'item.completed' && e.item?.type === 'command_execution' && e.item.exit_code === 0));
    assert.equal(first.findLast(e => e.type === 'item.completed' && e.item?.type === 'agent_message')?.item.text, nonce);
    const second = await invoke(true, 'Continue this conversation.');
    assert.equal(second.findLast(e => e.type === 'item.completed' && e.item?.type === 'agent_message')?.item.text, 'NATIVE_REOPENED');
    assert.equal(generations, compact ? 4 : 3);
    assert.equal(requests.filter(r => r.op === 'create').length, 1);
    assert.equal(requests.filter(r => r.op === 'rebuild').length, compact ? 3 : 1);
    assert.equal(requests.filter(r => r.op === 'open').length, compact ? 3 : 2);
    assert.equal(failures.length, 0, failures.join('\n'));
    const records = (await readdir(bindings)).filter(name => name.endsWith('.json'));
    assert.equal(records.length, 1);
    const record = JSON.parse(await readFile(join(bindings, records[0]), 'utf8'));
    assert.equal(record.in_flight, false);
    assert.equal(record.epoch, compact ? 4 : 3);
    assert.equal(record.model, 'xenocodex-sim');
    const reasoningItems = record.history.filter(item => item.type === 'reasoning');
    assert.equal(reasoningItems.length, compact ? 2 : 3, 'Reasoning persists across tool continuation and resume');
    assert.ok(reasoningItems.every(item => JSON.stringify(item).includes(thought)));
    const rolloutFiles = await readdir(join(home, 'sessions'), { recursive: true });
    const rollouts = await Promise.all(rolloutFiles.filter(name => name.endsWith('.jsonl')).map(name => readFile(join(home, 'sessions', name), 'utf8')));
    const events = rollouts.flatMap(text => text.trim().split('\n').map(line => JSON.parse(line)));
    assert.ok(events.some(event => event.type === 'response_item' && event.payload?.type === 'reasoning' && JSON.stringify(event.payload).includes(thought)), 'Raw reasoning is persisted as reasoning content');
    const turns = events.filter(event => event.type === 'event_msg' && event.payload?.type === 'task_started');
    assert.equal(turns.length, 2);
    assert.ok(turns.every(event => event.payload.model_context_window === Math.floor(65536 * 0.95)), 'Discovered context reaches Codex with its normal 5% headroom');
    assert.ok(events.filter(event => event.type === 'turn_context').every(event => event.payload.model === 'xenocodex-sim'), 'Discovered model is recorded in actual turn settings');
    if (compact) assert.ok(events.some(event => event.type === 'compacted'), 'Codex must commit a compaction checkpoint');
    assert.ok(events.some(event => event.type === 'session_meta' && event.payload?.model_provider === 'xenolith'));
    assert.ok(!first.concat(second).some(event => event.item?.type === 'agent_message' && event.item.text.includes(thought)), 'Reasoning must never become assistant answer text');
    console.log(`Native simulator artifacts: ${root}`);
  } finally {
    await writeFile(join(root, 'wire.json'), JSON.stringify(requests, null, 2));
    await writeFile(join(root, 'simulator-errors.json'), JSON.stringify(failures, null, 2));
  }
});
