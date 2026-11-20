import { spawnSync } from 'node:child_process';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { expect, it } from 'vitest';
import { installQuestionBirths } from '../src/host-births.js';
import { observeHost } from '../src/host-observer.js';
import { harnessIdentity, toPetEvents, type EventEnvelope } from '../src/events.js';

const host = process.env.PET_HARNESS_PATCHED_HOST;
const installationId = '11111111-1111-4111-8111-111111111111';

it.skipIf(host === undefined)('同一原问题服务跨写代次重载，精确终态匹配旧请求并保留新请求', async () => {
  const load = (path: string) => import(/* @vite-ignore */ pathToFileURL(join(host!, path, 'lib/index.js')).href);
  const [
    { Context }, { default: Sessions, Session, SessionId },
    { default: Projections }, { default: Agents }, { default: Questions }, Persistence,
  ] = await Promise.all([
    load('vendor/cordis'), load('packages/core/session'), load('packages/session/session-projection'),
    load('packages/core/agent'), load('packages/interaction/user-questions'), load('packages/session/session-persistence-jsonl'),
  ]);
  const root = await mkdtemp(join(tmpdir(), 'pet-direct-handoff-'));
  const ctx = new Context();
  const cancel = new AbortController();
  const events: EventEnvelope[] = [];
  try {
    await ctx.plugin(Sessions);
    await ctx.plugin(Projections);
    await ctx.plugin(Agents);
    await ctx.plugin(Questions);
    await ctx.plugin(Persistence, { root: join(root, 'sessions'), compression: 'none' });
    const id = SessionId('direct-handoff');
    const seed = Session.create(id);
    seed.append('session/end-seed', {});
    const initial = await ctx.sessionPersistence.create(seed.header);
    await initial.append(seed.snapshotEvents());
    await initial.close();

    const mount = () => ctx.plugin({
      inject: ['sessions', 'sessionProjections'],
      async apply(child: Parameters<typeof observeHost>[0]) {
        const births = await installQuestionBirths(child);
        const observer = observeHost(child, value => {
          events.push(...toPetEvents(value, harnessIdentity(value, installationId)));
        }, births.turn, births);
        await observer.ready;
        return () => observer.dispose();
      },
    });
    async function resume() {
      const writer = await ctx.sessionPersistence.open(id, 'write');
      const persisted = await writer.read();
      const proof = await writer.prepareObservationRecovery();
      const session = ctx.sessions.prepare(id, {
        meta: writer.header, inheritedEventCount: writer.inheritedEventCount, seed: persisted.events,
        eventState: persisted.eventState, observationRecovery: proof,
      });
      const detachSession = ctx.sessions.enter(session);
      ctx.sessions.announce(session);
      const agent = { id, session };
      const detachAgent = ctx.agents.enter(agent);
      return { agent, async close() { detachAgent(); detachSession(); await writer.close(); } };
    }

    const oldProvider = Promise.withResolvers<{ answers: [] }>();
    const newProvider = Promise.withResolvers<{ answers: [] }>();
    ctx.on('user-questions/request', (request: { questions: { id: string }[] }) =>
      request.questions[0]?.id === 'old' ? oldProvider.promise : newProvider.promise);
    const old = await resume();
    const first = await mount();
    const oldWork = ctx.userQuestions.ask({
      agent: old.agent, questions: [{ id: 'old', question: '旧题正文' }], signal: cancel.signal,
    });
    await expect.poll(() => events.length, { timeout: 1000 }).toBe(2);
    await first.dispose();
    const answer = { answers: [] as [] };
    oldProvider.resolve(answer);
    await expect(oldWork).resolves.toBe(answer);
    await old.close();

    const current = await resume();
    const second = await mount();
    const newWork = ctx.userQuestions.ask({
      agent: current.agent, questions: [{ id: 'new', question: '新题正文' }], signal: cancel.signal,
    });
    await expect.poll(() => events.length, { timeout: 1000 }).toBe(5);
    expect(events.map(event => event.kind.type)).toEqual([
      'source_recovery', 'attention_requested', 'source_recovery', 'attention_resolved', 'attention_requested',
    ]);
    expect(events[1]?.directQuestionOwner).toBeTruthy();
    expect(events[3]?.directQuestionOwner).toBe(events[1]?.directQuestionOwner);
    expect(events[3]?.identity.instanceId).toBe(events[1]?.identity.instanceId);
    expect(events[4]?.identity.instanceId).not.toBe(events[1]?.identity.instanceId);
    const sample = join(root, 'events.json');
    await writeFile(sample, JSON.stringify(events));
    const verify = spawnSync('cargo', [
      'test', '--locked', '-p', 'pet-core', '--test', 'harness_recovery', 'actual_direct_handoff', '--', '--ignored',
    ], { cwd: fileURLToPath(new URL('../../../', import.meta.url)), env: {
      ...process.env, PET_HARNESS_DIRECT_HANDOFF: sample,
    }, encoding: 'utf8' });
    expect(verify.status, verify.stdout + verify.stderr).toBe(0);
    expect(verify.stdout).toContain('1 passed');
    newProvider.resolve(answer);
    await expect(newWork).resolves.toBe(answer);
    await second.dispose();
    await current.close();
  } finally {
    cancel.abort();
    await ctx.fiber.dispose();
    await rm(root, { recursive: true, force: true });
  }
}, 60_000);
