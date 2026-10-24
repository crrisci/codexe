// Copyright (c) 2026 Francesco Pio Crisci. SPDX-License-Identifier: Apache-2.0
// Harness checks only; these do not load weights or claim visual accuracy.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createConnection, createServer } from 'node:net';
import { inflateSync } from 'node:zlib';
import { cliArgs, colors, frameReader, imageFixture, phaseEvidence, runClient, sha256, squarePng, wireObserver } from './helpers/vision-smoke.mjs';

async function temporary(t) {
  const root = await mkdtemp(join(tmpdir(), 'vision-harness-'));
  t.after(() => rm(root, { recursive: true, force: true }));return root;
}
test('vision fixtures contain the independently recorded color and a white border', async t => {
  const root = await temporary(t);
  for (const [, rgb] of colors) {
    const png = squarePng(rgb);assert.equal(png.subarray(0, 8).toString('hex'), '89504e470d0a1a0a');
    let offset = 8;const data = [];
    while (offset < png.length) {
      const size = png.readUInt32BE(offset), name = png.subarray(offset + 4, offset + 8).toString();
      if (name === 'IDAT') data.push(png.subarray(offset + 8, offset + 8 + size));offset += size + 12;
    }
    const pixels = inflateSync(Buffer.concat(data));
    assert.deepEqual([...pixels.subarray(1, 4)], [255, 255, 255]);
    assert.deepEqual([...pixels.subarray(128 * 769 + 1 + 128 * 3, 128 * 769 + 4 + 128 * 3)], rgb);
  }
  const a = await imageFixture(root, 'a.png'), b = await imageFixture(root, 'b.png', a.expected);
  assert.notEqual(a.expected, b.expected);assert.notEqual(a.sha256, b.sha256);
  assert.equal(sha256(await readFile(a.path)), a.sha256);
});
test('real CLI arguments close stdin and keep the prompt separate from variadic --image', async t => {
  const workspace = await temporary(t), prompt = 'Inspect this image without reading stdin.';
  const args = cliArgs({ workspace, prompt, image: join(workspace, 'fixture.png') });
  assert.equal(args.at(-2), '--');assert.equal(args.at(-1), prompt);
  assert.equal(args[args.indexOf('--image') + 2], '--');
  const result = await runClient(process.execPath, ['-e', "let text='';process.stdin.on('data',b=>text+=b);process.stdin.on('end',()=>console.log(JSON.stringify({args:process.argv.slice(1),stdin:text})));", '--', ...args], { timeoutMs: 10000 });
  assert.equal(result.code, 0, result.stderr);assert.equal(result.timed_out, false);
  assert.deepEqual(JSON.parse(result.stdout), { args, stdin: '' });
});
test('native frame observation handles fragmented UTF-8 and rejects corrupt/oversized frames', () => {
  const frames = [], errors = [], consume = frameReader(f => frames.push(f), e => errors.push(e));
  const bytes = Buffer.from('{"text":"caffè 🌋"}\n{"ok":true}\n');
  for (const byte of bytes) consume(Buffer.from([byte]));
  assert.deepEqual(frames, [{ text: 'caffè 🌋' }, { ok: true }]);assert.deepEqual(errors, []);
  const broken = frameReader(() => assert.fail(), e => errors.push(e), 8);broken(Buffer.from('123456789'));
  const malformed = frameReader(() => assert.fail(), e => errors.push(e));malformed(Buffer.from('{bad}\n'));
  assert.equal(errors.length, 2);
});
test('wire observer forwards original bytes in both directions without rewriting protocol', { timeout: 10000 }, async t => {
  const root = await temporary(t), nativePath = join(root, 'native.sock'), clientPath = join(root, 'client.sock');
  const native = createServer(socket => socket.on('data', data => socket.write(data)));
  await new Promise(resolve => native.listen(nativePath, resolve));
  const observed = [];
  const proxy = await wireObserver(clientPath, nativePath, record => observed.push(record));proxy.phase('fixture');
  t.after(async () => { await proxy.close();await new Promise(resolve => native.close(resolve)); });
  const original = Buffer.from(' { "payload": "à 🌋", "image": {"data":"AA=="} }\n');
  const received = await new Promise((resolve, reject) => {
    const socket = createConnection(clientPath);const chunks = [];let size = 0;
    socket.once('error', reject);socket.once('connect', () => {
      socket.write(original.subarray(0, 20));socket.write(original.subarray(20));
    });socket.on('data', bytes => { chunks.push(bytes);size += bytes.length;if (size === original.length) { socket.destroy();resolve(Buffer.concat(chunks)); } });
  });
  assert.deepEqual(received, original);assert.deepEqual(proxy.errors, []);
  assert.deepEqual(observed.map(o => o.direction), ['request', 'response']);
  assert.deepEqual(observed[0].frame, observed[1].frame);
});
function toolRecords() {
  const fixture = { path: '/fixture.png', sha256: sha256(Buffer.from('real bytes')) }, phase = 'tool';
  const r = (direction, frame, connection = 1) => ({ phase, direction, frame, connection });
  return { fixture, records: [r('request', { op: 'create' }), r('response', { ok: true, session: 'owned' }),
    r('response', { event: 'toolcall_end', id: 7, name: 'view_image', arguments: { path: fixture.path } }),
    r('response', { event: 'done', stop: 'tool_use' }), r('request', { op: 'open', session: 'owned' }, 2),
    r('request', { op: 'append', role: 'tool', call_id: 7, image: { data: Buffer.from('real bytes').toString('base64'), budget: 280 } }, 2),
    r('response', { event: 'done', stop: 'stop', usage: { cache_read: 0 } }, 2)] };
}
test('tool evidence binds actual bytes to the native call across reopened connections', () => {
  const { records, fixture } = toolRecords();
  const evidence = phaseEvidence(records, 'tool', fixture, { tool: true, budgets: [280] });
  assert.equal(evidence.images[0].session, 'owned');assert.equal(evidence.tool_calls.length, 1);
  for (const mutate of [
    copy => { copy[5].frame.image.data = Buffer.from('wrong image').toString('base64'); },
    copy => { copy[5].frame.call_id = 8; },
    copy => { copy[4].frame.session = 'unrelated'; },
    copy => { copy[5].frame.image.budget = 70; },
    copy => { copy[2].frame.name = 'exec_command'; },
    copy => { copy.splice(1, 0, { phase: 'tool', direction: 'request', connection: 1, frame: { op: 'append', image: { data: 'AA==' } } }); },
  ]) { const copy = structuredClone(records);mutate(copy);assert.throws(() => phaseEvidence(copy, 'tool', fixture, { tool: true, budgets: [280] })); }
});
test('a plausible answer or tool-call name without returned image bytes cannot pass', () => {
  const { records, fixture } = toolRecords();records.splice(5, 1);
  assert.throws(() => phaseEvidence(records, 'tool', fixture, { tool: true, budgets: [280] }));
  assert.throws(() => phaseEvidence([], 'attachment', fixture, { attachment: true, budgets: [280] }));
});

test('missing explicit inputs produce a preserved failed report without starting an engine',
  { skip: process.platform !== 'linux', timeout: 20000 }, async () => {
    const env = { ...process.env };
    for (const name of ['XENOLITH_BIN', 'XENOLITH_MODEL', 'XENOLITH_MMPROJ', 'XENOCODEX_NATIVE_CODEX_BIN']) delete env[name];
    const script = new URL('./vision-smoke.mjs', import.meta.url);
    const result = await runClient(process.execPath, [script.pathname], { env, timeoutMs: 15000 });
    assert.equal(result.code, 1);assert.equal(result.timed_out, false);
    const root = /^VISION_ROOT (.+)$/m.exec(result.stdout)?.[1];assert.ok(root, result.stderr);
    const report = JSON.parse(await readFile(join(root, 'report.json'), 'utf8'));
    assert.equal(report.status, 'failed');assert.match(report.error, /XENOLITH_BIN/);
    assert.deepEqual(report.engine_pids, []);assert.deepEqual(report.cases, []);
  });

test('interruption kills the owned client group including a descendant retaining output pipes',
  { skip: process.platform === 'win32', timeout: 20000 }, async t => {
    const root = await temporary(t), ready = join(root, 'ready'), control = new AbortController();
    const script = `const {spawn}=require('node:child_process');const child=spawn(process.execPath,['-e','setInterval(()=>{},1000)'],{stdio:['ignore',1,2]});require('node:fs').writeFileSync(${JSON.stringify(ready)},String(child.pid));setInterval(()=>{},1000);`;
    const pending = runClient(process.execPath, ['-e', script], { signal: control.signal, timeoutMs: 15000 });
    t.after(async () => { control.abort();await pending; });
    let pid;
    const deadline = Date.now() + 10000;
    while (Date.now() < deadline) {
      try { pid = Number(await readFile(ready, 'utf8'));break; }
      catch (error) { if (error.code !== 'ENOENT') throw error;await new Promise(resolve => setTimeout(resolve, 25)); }
    }
    assert.ok(Number.isInteger(pid) && pid > 0);control.abort();
    const result = await pending;assert.equal(result.aborted, true);assert.equal(result.timed_out, false);
    for (let attempt = 0; ; attempt++) {
      try {
        if (process.platform === 'linux') {
          if (/^\d+ \(.*\) Z /.test(await readFile(`/proc/${pid}/stat`, 'utf8'))) break;
        } else process.kill(pid, 0);
      } catch (error) { if (['ENOENT', 'ESRCH'].includes(error.code)) break;throw error; }
      assert.ok(attempt < 80, 'Owned descendant must terminate');
      await new Promise(resolve => setTimeout(resolve, 25));
    }
  });
