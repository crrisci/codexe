// Copyright (c) 2026 Francesco Pio Crisci. SPDX-License-Identifier: Apache-2.0
// No weights: verify the fault boundary itself, never infer real recovery here.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createConnection, createServer } from 'node:net';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { branchAnswer, faultProxy } from './helpers/vision-recovery.mjs';
import { cliArgs, frameReader, runClient } from './helpers/vision-smoke.mjs';

for (const mode of ['lost-tool-image-ack', 'before-image-ack']) test(`fault injection identifies ${mode} without fabricating an ACK`, { timeout: 10000 }, async t => {
  const root = await mkdtemp(join(tmpdir(), 'vision-fault-')), sockets = new Set();
  const requests = [], records = [], replies = [];let interrupted = false, replyTimer;
  const upstream = createServer(socket => {
    sockets.add(socket);socket.on('error', () => {}).on('close', () => sockets.delete(socket));
    socket.on('data', frameReader(frame => {
      requests.push(frame);
      if (!frame.image) socket.write('{"ok":true}\n');
      else replyTimer = setTimeout(() => socket.write('{"ok":true,"marker":9}\n'), mode === 'before-image-ack' ? 200 : 0);
    }, error => assert.fail(error)));
  });
  await new Promise(resolve => upstream.listen(join(root, 'engine.sock'), resolve));
  const proxy = await faultProxy(join(root, 'client.sock'), join(root, 'engine.sock'), {
    mode, record: r => records.push(r), interrupt() { interrupted = true; },
  });
  const client = createConnection(join(root, 'client.sock'));
  t.after(async () => {
    clearTimeout(replyTimer);client.destroy();await proxy.close();await proxy.close();
    for (const socket of sockets) socket.destroy();
    await new Promise(resolve => upstream.close(resolve));await rm(root, { recursive: true, force: true });
  });
  client.on('data', frameReader(frame => replies.push(frame), error => assert.fail(error)));
  await new Promise(resolve => client.once('connect', resolve));
  client.write('{"op":"describe"}\n');
  const wait = async predicate => { const end = Date.now() + 3000;while (!predicate()) { assert.ok(Date.now() < end);await new Promise(r => setTimeout(r, 5)); } };
  await wait(() => replies.length === 1);
  client.write('{"op":"append","role":"tool","call_id":7,"image":{"data":"YWJj","budget":280}}\n');
  await wait(() => interrupted);
  assert.equal(requests.length, 2);assert.equal(requests[1].image.data, 'YWJj');
  assert.equal(proxy.faults.length, 1);assert.deepEqual(proxy.errors, []);assert.equal(replies.length, 1);
  const acknowledgments = records.filter(r => r.direction === 'response' && r.frame.marker === 9);
  assert.equal(acknowledgments.length, mode === 'lost-tool-image-ack' ? 1 : 0);
});

test('recovery addresses an explicit parent thread after a fork', () => {
  const args = cliArgs({ workspace: '/work', resume: 'parent-thread', prompt: 'Continue.' });
  assert.equal(args[args.indexOf('resume') + 1], 'parent-thread');assert.ok(!args.includes('--last'));
});


test('fork evidence permits user input notifications and rejects tool effects', () => {
  assert.equal(branchAnswer([{ type: 'userMessage' }, { type: 'reasoning' }, { type: 'agentMessage', text: 'BLUE' }]), 'BLUE');
  for (const type of ['commandExecution', 'imageView', 'mcpToolCall', 'unknown'])
    assert.throws(() => branchAnswer([{ type }, { type: 'agentMessage', text: 'BLUE' }]));
});


test('recovery preflight requires its session tool before loading weights', { skip: process.platform !== 'linux', timeout: 20000 }, async () => {
  const env = { ...process.env, XENOLITH_BIN: process.execPath, XENOLITH_MODEL: process.execPath,
    XENOLITH_MMPROJ: process.execPath, XENOCODEX_NATIVE_CODEX_BIN: process.execPath };
  delete env.XENOCODEX_NATIVE_SESSION_BIN;
  const result = await runClient(process.execPath, [new URL('./vision-smoke.mjs', import.meta.url).pathname, '--recovery'], { env, timeoutMs: 15000 });
  assert.equal(result.code, 1);assert.equal(result.timed_out, false);
  const root = /^VISION_ROOT (.+)$/m.exec(result.stdout)?.[1];assert.ok(root, result.stderr);
  const report = JSON.parse(await readFile(join(root, 'report.json'), 'utf8'));
  assert.equal(report.mode, 'real_vision_recovery');assert.equal(report.status, 'failed');
  assert.match(report.error, /XENOCODEX_NATIVE_SESSION_BIN/);assert.deepEqual(report.engine_pids, []);
});
