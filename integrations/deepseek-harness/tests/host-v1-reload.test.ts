import { spawnSync } from 'node:child_process';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { expect, it } from 'vitest';
import { harnessIdentity, toPetEvents, type EventEnvelope, type HarnessObservation, type ObservationGeneration } from '../src/events.js';
import { installQuestionBirths } from '../src/host-births.js';
import { observeHost } from '../src/host-observer.js';

const host = process.env.PET_HARNESS_PATCHED_HOST;
const installationId = '11111111-1111-4111-8111-111111111111';

it.skipIf(host === undefined)('保留仍活动的 v1 写者，重载后继续向旧检查点投递回合和真实直接答复', async () => {
  const load = (path: string) => import(/* @vite-ignore */ pathToFileURL(join(host!, path, 'lib/index.js')).href);
  const [
    { Context }, { default: Sessions, Session, SessionId, SessionLogOffset },
    { default: Projections }, { default: Agents }, { default: Questions },
  ] = await Promise.all([
    load('vendor/cordis'), load('packages/core/session'), load('packages/session/session-projection'),
    load('packages/core/agent'), load('packages/interaction/user-questions'),
  ]);
  const root = await mkdtemp(join(tmpdir(), 'pet-v1-reload-'));
  const ctx = new Context();
  const controller = new AbortController();
  const serialize = (value: HarnessObservation) => toPetEvents(value, harnessIdentity(value, installationId));
  const current: EventEnvelope[] = [];
  try {
    await ctx.plugin(Sessions);
    await ctx.plugin(Projections);
    await ctx.plugin(Agents);
    await ctx.plugin(Questions);
    const id = SessionId('legacy-active-writer');
    const seed = Session.create(id);
    seed.append('turn/start', { turn: 1 });
    seed.append('turn/end', { turn: 1, reason: { kind: 'completed' } });
    seed.append('session/end-seed', {});
    const proof: ObservationGeneration = {
      version: 1, generation: 2, streamId: '33333333-3333-4333-8333-333333333333',
      epochId: '44444444-4444-4444-8444-444444444444', recoveredEventCount: seed.seq,
      ancestors: [{ epochId: '55555555-5555-4555-8555-555555555555', generation: 1, firstSequence: 1 }],
    };
    const session = ctx.sessions.prepare(id, {
      meta: seed.header, inheritedEventCount: SessionLogOffset(0), seed: seed.snapshotEvents(),
      eventState: 'detached', observationRecovery: proof,
    });
    ctx.sessions.enter(session);
    ctx.sessions.announce(session);
    const agent = { id, session };
    ctx.agents.enter(agent);
    const start = session.append('turn/start', { turn: 2 });
    const provider = Promise.withResolvers<{ answers: [] }>();
    ctx.on('user-questions/request', () => provider.promise);
    const answer = ctx.userQuestions.ask({ agent, questions: [{ id: 'fixture', question: '原服务题目' }], signal: controller.signal });
    const audit = ctx.userQuestions.observeDirectQuestions(() => {});
    const request = audit.snapshot.active[0];
    audit.dispose();
    expect(request).toBeDefined();
    const source = { sessionId: id, sessionCreatedAt: seed.header.createdAt, observation: proof };
    const old = [
      ...serialize({ ...source, time: start.time, type: 'turn/start', turn: 2, seq: start.seq }),
      ...serialize({
        ...source, time: request.startedAt, type: 'question/requested', lifetime: 'until_resolved',
        turn: request.birthTurn ?? undefined, requestId: `question:${request.id}`, observationId: request.id,
      }),
    ];

    const observer = await ctx.plugin({ inject: ['sessions', 'sessionProjections'], async apply(child: Parameters<typeof observeHost>[0]) {
      const births = await installQuestionBirths(child);
      const adapter = observeHost(child, value => { current.push(...serialize(value)); }, births.turn, births);
      await adapter.ready;
      return () => adapter.dispose();
    } });
    await expect.poll(() => current.some(event => event.kind.type === 'attention_requested')).toBe(true);
    const result = { answers: [] as [] };
    provider.resolve(result);
    await expect(answer).resolves.toBe(result);
    session.append('turn/end', { turn: 2, reason: { kind: 'completed' } });
    session.append('turn/start', { turn: 3 });
    await expect.poll(() => current.length).toBe(5);
    await observer.dispose();
    expect(current.every(event => event.schemaVersion === 3 && event.identity.instanceId === proof.epochId)).toBe(true);
    const recovered = current.find(event => event.kind.type === 'source_recovery');
    expect(recovered?.observation).toEqual(proof);
    expect(recovered?.kind).toMatchObject({ type: 'source_recovery', running: true });
    expect(recovered?.kind).not.toHaveProperty('timed');
    const active = current.find(event => event.kind.type === 'attention_requested');
    expect(active?.eventId).toBe(old[1]?.eventId);
    const ended = current.find(event => event.kind.type === 'attention_resolved');
    expect(ended?.directQuestionOwner).toBe(audit.snapshot.ownerId);
    const sample = join(root, 'events.json');
    await writeFile(sample, JSON.stringify({ old, current }));
    const verify = spawnSync('cargo', [
      'test', '--locked', '-p', 'pet-core', '--test', 'harness_recovery', 'actual_v1_observer_reload', '--', '--ignored',
    ], { cwd: fileURLToPath(new URL('../../../', import.meta.url)), env: { ...process.env, PET_HARNESS_V1_RELOAD: sample }, encoding: 'utf8' });
    expect(verify.status, verify.stdout + verify.stderr).toBe(0);
    expect(verify.stdout).toContain('1 passed');
  } finally {
    controller.abort();
    await ctx.fiber.dispose();
    await rm(root, { recursive: true, force: true });
  }
}, 60_000);
