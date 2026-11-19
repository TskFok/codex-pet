import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtemp, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { expect, it } from 'vitest';
import * as plugin from '../src/index.js';
import type { EventEnvelope } from '../src/events.js';

const host = process.env.PET_HARNESS_PATCHED_HOST;
const installationId = '11111111-1111-4111-8111-111111111111';

it.skipIf(host === undefined || process.platform === 'win32')('真实写句柄崩溃回退，经生产适配器恢复原定时问题并识别旧代', async () => {
  const repo = fileURLToPath(new URL('../../../', import.meta.url));
  const integration = fileURLToPath(new URL('../', import.meta.url));
  const bridgePath = join(repo, 'target', 'debug', 'pet-bridge');
  const adapter = join(integration, 'dist');
  execFileSync('cargo', ['build', '--locked', '-q', '-p', 'pet-bridge'], { cwd: repo, stdio: 'pipe' });
  const root = await mkdtemp(join(tmpdir(), 'pet-generation-'));
  const data = join(root, 'sessions');
  const probeDir = join(root, 'probe');
  const fixture = join(root, 'observations.json');
  const load = (path: string) => import(/* @vite-ignore */ pathToFileURL(join(host!, path, 'lib/index.js')).href);
  const [
    { Context }, { default: Sessions, Session, SessionId }, { default: Projections }, { default: Questions }, Persistence, llm,
  ] = await Promise.all([
    load('vendor/cordis'), load('packages/core/session'), load('packages/session/session-projection'),
    load('packages/interaction/user-questions'), load('packages/session/session-persistence-jsonl'), load('packages/llm/llm'),
  ]);
  const ctx = new Context();
  async function events(): Promise<EventEnvelope[]> {
    const names = await readdir(probeDir).catch((error: NodeJS.ErrnoException) => {
      if (error.code === 'ENOENT') return [];
      throw error;
    });
    return Promise.all(names.filter((name: string) => name.endsWith('.json')).map(async (name: string) =>
      JSON.parse(await readFile(join(probeDir, name), 'utf8'))));
  }
  try {
    await ctx.plugin(Sessions);
    await ctx.plugin(Projections);
    await ctx.plugin(Questions);
    await ctx.plugin(Persistence, { root: data, compression: 'none' });
    const id = SessionId('generation-roundtrip');
    const seed = Session.create(id);
    seed.append('turn/start', { turn: 1 });
    seed.append('step/start', { turn: 1, step: 1 });
    seed.append('request/header', {
      header: {
        config: { provider: 'synthetic', model: 'offline-fixture' },
        tools: [{ name: 'ask_user_question', description: 'fixture', parameters: { type: 'object', properties: { timeout: { type: 'integer' } } } }],
      },
      reason: 'initial',
    });
    const callId = llm.ToolCallId('retained');
    const questionArguments = JSON.stringify({ questions: [{ id: 'scope', question: '仅保存在宿主' }] });
    seed.append('assistant/message', {
      turn: 1, step: 1, stream: [],
      message: llm.createAssistantMessage({
        source: { kind: 'model', provider: 'synthetic', model: 'offline-fixture' },
        content: [{ type: 'tool-call', id: callId, name: 'ask_user_question', arguments: questionArguments }],
      }),
    }, { surfaceOp: 'append' });
    seed.append('tool/call', {
      turn: 1, step: 1, callId, name: 'ask_user_question', arguments: questionArguments,
    });
    seed.append('tool/result', {
      turn: 1, step: 1, message: llm.createToolResultMessage({
        callId, content: [{ type: 'text', text: '{"pending":true}' }], isError: false,
      }),
    }, { surfaceOp: 'append' });
    seed.append('step/end', { turn: 1, step: 1 });
    seed.append('turn/end', { turn: 1, reason: { kind: 'completed' } });
    seed.append('session/end-seed', {});
    const writer = await ctx.sessionPersistence.create(seed.header);
    await writer.append(seed.snapshotEvents());
    await writer.close();

    const child = spawnSync(process.execPath, [
      fileURLToPath(new URL('./fixtures/crashed-observation.mjs', import.meta.url)), host!, data, id, adapter, installationId,
    ], { encoding: 'utf8', timeout: 15_000 });
    expect(child.signal, child.stderr).toBe('SIGKILL');
    const old: EventEnvelope[] = JSON.parse(child.stdout);
    expect(old.map(event => event.kind.type)).toEqual(['source_recovery', 'turn_started', 'turn_ended']);

    const pluginFiber = await ctx.plugin(plugin, { bridgePath, probeDir, installationId });
    const restored = await ctx.sessionPersistence.open(id, 'write');
    const persisted = await restored.read();
    expect(persisted.events).toHaveLength(seed.seq);
    const proof = await restored.prepareObservationRecovery();
    const session = ctx.sessions.prepare(id, {
      meta: restored.header, inheritedEventCount: restored.inheritedEventCount, seed: persisted.events,
      eventState: persisted.eventState, observationRecovery: proof,
    });
    ctx.sessions.enter(session);
    ctx.sessions.announce(session);
    session.append('turn/start', { turn: 2 });
    session.append('user/message', llm.createUserMessage({
      source: { kind: 'user-question-reply', callId, outcome: 'answered' }, content: [{ type: 'text', text: '仅原应用答案' }],
    }), { surfaceOp: 'append' });
    await expect.poll(async () => (await events()).length, { timeout: 5_000 }).toBe(3);
    await pluginFiber.dispose();
    const current = await events();
    const currentSnapshot = current.find(event => event.kind.type === 'source_recovery');
    const currentStart = current.find(event => event.kind.type === 'turn_started');
    const currentResolution = current.find(event => event.kind.type === 'attention_resolved');
    expect(currentSnapshot).toMatchObject({
      schemaVersion: 3, observation: { generation: 2, recoveredEventCount: seed.seq },
      kind: { timed: [{ requestId: 'question:retained', birthSequence: 4, turnId: '1' }] },
    });
    expect(currentStart?.sequence).toBe(old[1]?.sequence);
    expect(currentStart?.eventId).not.toBe(old[1]?.eventId);
    expect(currentResolution).toMatchObject({
      turnId: '1', questionBirth: 4, kind: { requestId: 'question:retained' },
    });
    expect(JSON.stringify([old, current])).not.toContain('仅保存在宿主');
    expect(JSON.stringify([old, current])).not.toContain('仅原应用答案');
    for (const value of old) {
      const result = spawnSync(bridgePath, ['harness', '--probe-dir', probeDir], { input: JSON.stringify(value), encoding: 'utf8' });
      expect(result.status, result.stderr).toBe(0);
    }
    await writeFile(fixture, JSON.stringify({ old, current }));
    const result = spawnSync('cargo', [
      'test', '--locked', '-p', 'pet-core', '--test', 'harness_recovery', 'actual_jsonl', '--', '--ignored',
    ], { cwd: repo, env: { ...process.env, PET_HARNESS_RECOVERY_EVENTS: fixture }, encoding: 'utf8' });
    expect(result.status, result.stdout + result.stderr).toBe(0);
    expect(result.stdout).toContain('1 passed');
  } finally {
    await ctx.fiber.dispose();
    await rm(root, { recursive: true, force: true });
  }
}, 120_000);
