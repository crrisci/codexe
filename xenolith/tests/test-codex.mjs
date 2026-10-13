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

test('native Codex executes a tool, appends its result and reopens a persistent binding', { timeout: 120000 }, async t => {
  const parent = resolve('.test-artifacts');
  await mkdir(parent, { recursive: true });
  const root = await mkdtemp(join(parent, 'native-'));
  const workspace = join(root, 'workspace');
  const home = join(root, 'codex-home');
  const bindings = join(root, 'bindings');
  await Promise.all([mkdir(workspace), mkdir(home)]);
  const nonce = `NATIVE_${randomBytes(16).toString('hex')}`;
  await writeFile(join(workspace, 'input.txt'), nonce + '\n');
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
          if (r.op === 'describe') send({ ok: true, protocol: 1, model: 'xenocodex-sim', context_window: 65536,
            max_output: 8192, max_frame: 1048576, kvstore: true,
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
            send({ ok: true, marker: 0 });
          } else if (r.op === 'append') {
            if (r.role === 'tool') {
              assert.equal(r.call_id, 17);
              assert.ok(r.text.includes(nonce), 'Result must come from the actual executed tool');
              sessions.get(session).pending = [];
            } else assert.equal(r.role, 'user');
            send({ ok: true, marker: sessions.get(session).marker });
          } else if (r.op === 'rewind_cost') {
            send({ ok: true, prefill: 50 });
          } else if (r.op === 'generate') {
            const state = sessions.get(session);
            assert.ok(state);
            assert.equal(r.reasoning, false);
            generations++;
            assert.ok(generations <= 3, 'No unexpected inference retries');
            send({ event: 'start' });
            send({ event: 'inference_progress', phase: 'prefill', state: 'running', tokens: 0, total: 10, elapsed_ms: 0 });
            send({ event: 'inference_progress', phase: 'prefill', state: 'finished', tokens: 10, total: 10, elapsed_ms: 5 });
            if (generations === 1) {
              assert.ok(state.tools.some(tool => tool.name === 'exec_command'), 'Actual Codex shell tool declaration required');
              send({ event: 'toolcall_start', id: 17, name: 'exec_command' });
              send({ event: 'toolcall_end', id: 17, name: 'exec_command', arguments: { cmd: 'cat input.txt', login: false, max_output_tokens: 1000, yield_time_ms: 1000 } });
              state.pending = [17];
            } else send({ event: 'text_delta', text: generations === 2 ? nonce : 'NATIVE_REOPENED' });
            state.marker++;
            send({ event: 'done', stop: generations === 1 ? 'tool_use' : 'stop', marker: state.marker,
              usage: { input: 6, cache_read: 600, output: 3, total: 609, reasoning: 0, replayed: 0 },
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
    model_provider: 'xenolith', model_context_window: 65536, model_reasoning_effort: 'none', web_search: 'disabled',
    'model_providers.xenolith.name': 'Xenolith', 'model_providers.xenolith.base_url': `unix://${socketPath}`,
    'model_providers.xenolith.wire_api': 'xenolith', 'model_providers.xenolith.requires_openai_auth': false,
    'model_providers.xenolith.supports_websockets': false, 'model_providers.xenolith.stream_max_retries': 0,
    'model_providers.xenolith.request_max_retries': 0,
  };
  const flags = Object.entries(config).flatMap(([key, value]) => ['-c', `${key}=${JSON.stringify(value)}`]);
  async function invoke(resume, instruction) {
    const args = ['--no-daemon', 'exec', '--ignore-user-config', '--ignore-rules',
      '--skip-git-repo-check', '--sandbox', 'read-only', '--json', '-C', workspace, '-m', 'xenocodex-sim',
      ...flags, ...(resume ? ['resume', '--last'] : []), instruction];
    const child = spawn(binary, args, { cwd: workspace, env: { ...process.env, CODEX_HOME: home, XENOLITH_STATE_DIR: bindings }, stdio: ['ignore', 'pipe', 'pipe'] });
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
    const first = await invoke(false, 'Read input.txt using exec_command and report its exact contents.');
    assert.ok(first.some(e => e.type === 'item.completed' && e.item?.type === 'command_execution' && e.item.exit_code === 0));
    assert.equal(first.findLast(e => e.type === 'item.completed' && e.item?.type === 'agent_message')?.item.text, nonce);
    const second = await invoke(true, 'Continue this conversation.');
    assert.equal(second.findLast(e => e.type === 'item.completed' && e.item?.type === 'agent_message')?.item.text, 'NATIVE_REOPENED');
    assert.equal(generations, 3);
    assert.equal(requests.filter(r => r.op === 'create').length, 1);
    assert.equal(requests.filter(r => r.op === 'rebuild').length, 1);
    assert.equal(requests.filter(r => r.op === 'open').length, 2);
    assert.equal(failures.length, 0, failures.join('\n'));
    const records = (await readdir(bindings)).filter(name => name.endsWith('.json'));
    assert.equal(records.length, 1);
    const record = JSON.parse(await readFile(join(bindings, records[0]), 'utf8'));
    assert.equal(record.in_flight, false);
    assert.equal(record.epoch, 3);
    console.log(`Native simulator artifacts: ${root}`);
  } finally {
    await writeFile(join(root, 'wire.json'), JSON.stringify(requests, null, 2));
    await writeFile(join(root, 'simulator-errors.json'), JSON.stringify(failures, null, 2));
  }
});
