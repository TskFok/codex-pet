import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { expect, it } from 'vitest';
import { installQuestionBirths } from '../src/host-births.js';
import { observeHost } from '../src/host-observer.js';
import type { HarnessObservation } from '../src/events.js';

const host = process.env.PET_HARNESS_PATCHED_HOST;
const epochId = '22222222-2222-4222-8222-222222222222';

it.skipIf(host === undefined).each([1, 2] as const)('凭证 v%i 下纯 PTC 的前台问题、重载和迟答共用日志出生标识', async version => {
  const load = (path: string) => import(/* @vite-ignore */ pathToFileURL(join(host!, path, 'lib/index.js')).href);
  const [
    { Context }, { default: Sessions, SessionId, SESSION_FORMAT_VERSION, SessionLogOffset },
    { default: Projections }, { default: Agents }, { default: Questions, UserQuestionError }, { ToolCallId },
  ] = await Promise.all([
    load('vendor/cordis'), load('packages/core/session'), load('packages/session/session-projection'),
    load('packages/core/agent'), load('packages/interaction/user-questions'), load('packages/llm/llm'),
  ]);
  const ctx = new Context();
  const cancel = new AbortController();
  const observed: HarnessObservation[] = [];
  try {
    await ctx.plugin(Sessions);
    await ctx.plugin(Projections);
    await ctx.plugin(Agents);
    await ctx.plugin(Questions);
    const id = SessionId('ptc-question');
    const session = ctx.sessions.prepare(id, {
      seed: [], eventState: 'detached', inheritedEventCount: SessionLogOffset(0),
      meta: { version: SESSION_FORMAT_VERSION, id, createdAt: 100, isSeeded: false },
      observationRecovery: {
        ...(version === 1 ? { version } : { version, firstLiveSequence: 0 }),
        streamId: '11111111-1111-4111-8111-111111111111', epochId,
        generation: 1, recoveredEventCount: 0, ancestors: [],
      },
    });
    ctx.sessions.enter(session);
    ctx.sessions.announce(session);
    const agent = {
      id, session, inbox: { nextTurn: [], nextStep: [] },
      steer(message: unknown) { session.append('user/message', message, { surfaceOp: 'append' }); },
    };
    ctx.agents.enter(agent);
    const mount = () => ctx.plugin({ inject: ['sessions','sessionProjections'], async apply(child: Parameters<typeof observeHost>[0]) {
      const births = await installQuestionBirths(child);
      const observer = observeHost(child, value => { observed.push(value); }, births.turn, births);
      await observer.ready;
      return () => observer.dispose();
    } });

    let plugin = await mount();
    session.append('turn/start', { turn: 1 });
    session.append('request/header', {
      header: { config: { provider: 'synthetic', model: 'offline' },
        tools: [{ name: 'run_code', description: '离线夹具', parameters: { type: 'object' } }] },
      reason: 'initial',
    });
    const rootCallId = ToolCallId('run');
    const subCallId = ToolCallId('run:ptc:1');
    const questions = [{ id: 'one', question: '不转发正文' }];
    const dispatch = { rootCallId, parentCallId: rootCallId, subCallId, name: 'ask_user_question', arguments: { questions } };
    const birth = session.append('tool/ptc-dispatch-start', dispatch);
    const originalFrontend = Promise.withResolvers<never>();
    ctx.on('user-questions/request', () => originalFrontend.promise);
    const wait = ctx.userQuestions.askTimed({ agent, questions, signal: cancel.signal }, subCallId, 30_000);
    const scope = `question:${epochId}:${birth.seq}:${subCallId}`;
    const first = observed.find(event => event.type === 'question/requested');
    await plugin.dispose();
    plugin = await mount();
    const foreground = observed.filter(event => event.type === 'session/recovered').at(-1);
    originalFrontend.reject(new UserQuestionError('原前台超时', 'ASK_TIMED_OUT'));
    await expect(wait).resolves.toEqual({ pending: true, callId: subCallId });
    session.append('tool/ptc-dispatch', { ...dispatch, isError: false, content: [{ type: 'text', text: '{"pending":true}' }] });
    await plugin.dispose();
    plugin = await mount();
    const continued = observed.filter(event => event.type === 'session/recovered').at(-1);
    expect(ctx.userQuestions.answer(agent, subCallId, { answers: [{ id: 'one', selected: ['原回答'] }] })).toBe(true);
    expect(first).toMatchObject({ requestId: scope, questionBirth: birth.seq, turn: 1 });
    expect(foreground?.recoverySnapshot).not.toHaveProperty('timed');
    expect(continued?.recoverySnapshot?.timed).toEqual([{ requestId: scope, birthSequence: birth.seq, turnId: '1' }]);
    expect(observed.filter(event => event.type === 'question/resolved')).toEqual([
      expect.objectContaining({ requestId: scope, questionBirth: birth.seq, turn: 1 }),
    ]);
    await plugin.dispose();
  } finally {
    cancel.abort();
    await ctx.fiber.dispose();
  }
});
