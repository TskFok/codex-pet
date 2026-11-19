import type { Context as HostContext } from '@deepseek-ai/cordis';
import type { AskUserQuestionAnswer, AskUserQuestionRequestEvent } from '@deepseek-ai/dsh-user-questions/types';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdir, mkdtemp, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { expect, it } from 'vitest';
import * as plugin from '../src/index.js';
import type { EventEnvelope } from '../src/events.js';

const host = process.env.PET_HARNESS_PATCHED_HOST;

it.skipIf(host === undefined)('已构建的原问题服务跨插件重载恢复 direct 与 idle 请求，经真实桥接和日志去重', async () => {
  const root = fileURLToPath(new URL('../../../', import.meta.url));
  execFileSync('cargo', ['build', '--locked', '-q', '-p', 'pet-bridge'], { cwd: root, stdio: 'pipe' });
  const load = (relative: string) => import(/* @vite-ignore */ pathToFileURL(join(host!, relative, 'lib/index.js')).href);
  const [{ Context }, { default: Sessions, SessionId }, { default: Projections }, { default: Agents }, questions] = await Promise.all([
    load('vendor/cordis'),
    load('packages/core/session'),
    load('packages/session/session-projection'),
    load('packages/core/agent'),
    load('packages/interaction/user-questions'),
  ]);
  const directory = await mkdtemp(join(tmpdir(), 'pet-direct-recovery-'));
  const context: HostContext = new Context();
  const properties = {
    bridgePath: join(root, 'target', 'debug', process.platform === 'win32' ? 'pet-bridge.exe' : 'pet-bridge'),
    installationId: '11111111-1111-4111-8111-111111111111',
  };
  async function probe(name: string): Promise<EventEnvelope[]> {
    const path = join(directory, name);
    return Promise.all((await readdir(path)).filter(file => file.endsWith('.json')).map(async file =>
      JSON.parse(await readFile(join(path, file), 'utf8'))));
  }
  try {
    await mkdir(join(directory, 'first'), { mode: 0o700 });
    await mkdir(join(directory, 'resumed'), { mode: 0o700 });
    await context.plugin(Sessions);
    await context.plugin(Projections);
    await context.plugin(Agents);
    await context.plugin(questions.default);
    const session = context.sessions.create(SessionId('direct-or-idle'));
    const agent = { id: session.id, session } as NonNullable<AskUserQuestionRequestEvent['agent']>;
    const service: { ask(request: AskUserQuestionRequestEvent): Promise<AskUserQuestionAnswer> } = context.get('userQuestions');
    const registry = context.get('agents');
    expect(registry).toBeDefined();
    // The unit fixture owns only registry identity. No model, queue, or agent driver is mounted.
    registry!.enter(agent, false);
    const first = await context.plugin(plugin, { ...properties, probeDir: join(directory, 'first') });
    const waiting = Promise.withResolvers<AskUserQuestionAnswer>();
    const cancelled = Promise.withResolvers<AskUserQuestionAnswer>();
    const invalid = Promise.withResolvers<AskUserQuestionAnswer>();
    const independent = Promise.withResolvers<AskUserQuestionAnswer>();
    const answers = new Map([
      ['direct', waiting], ['idle', cancelled], ['invalid', invalid], ['independent', independent],
    ]);
    context.on('user-questions/request', request => answers.get(request.questions[0]?.id ?? '')!.promise);
    session.append('turn/start', { turn: 1 });
    const directWork = service.ask({ agent, questions: [{ id: 'direct', question: '私密原问题' }] });
    session.append('turn/end', { turn: 1, reason: { kind: 'completed' } });
    const idleWork = service.ask({ agent, questions: [{ id: 'idle', question: '私密空闲问题' }] });
    await expect.poll(async () => (await probe('first')).length, { timeout: 5_000 }).toBe(4);
    await first.dispose();

    const originalError = new questions.UserQuestionError('cancelled', 'ASK_CANCELLED');
    cancelled.reject(originalError);
    await expect(idleWork).rejects.toBe(originalError);
    const additionalAsk = (id: string, request: string) => {
      const session = context.sessions.create(SessionId(id));
      const owner = { id: session.id, session } as NonNullable<AskUserQuestionRequestEvent['agent']>;
      registry!.enter(owner, false);
      return service.ask({ agent: owner, questions: [{ id: request, question: '私密原问题' }] });
    };
    const invalidWork = additionalAsk('  ', 'invalid');
    const independentWork = additionalAsk('独立会话', 'independent');
    const resumed = await context.plugin(plugin, { ...properties, probeDir: join(directory, 'resumed') });
    await expect.poll(async () => (await probe('resumed')).some(event =>
      event.sessionId === '独立会话' && event.kind.type === 'attention_requested'), { timeout: 3_000 }).toBe(true);
    const originalAnswer = { answers: [{ id: 'direct', selected: ['私密原回答'] }] };
    waiting.resolve(originalAnswer);
    await expect(directWork).resolves.toBe(originalAnswer);
    invalid.resolve(originalAnswer);
    independent.resolve(originalAnswer);
    await expect(invalidWork).resolves.toBe(originalAnswer);
    await expect(independentWork).resolves.toBe(originalAnswer);
    await expect.poll(async () => (await probe('resumed')).length, { timeout: 5_000 }).toBe(5);
    await resumed.dispose();

    const firstEvents = await probe('first');
    const recovered = await probe('resumed');
    expect(recovered.some(event => event.sessionId.trim() === '')).toBe(false);
    const other = recovered.filter(event => event.sessionId === '独立会话');
    expect(other.map(event => event.kind.type)).toEqual(expect.arrayContaining(['attention_requested', 'attention_resolved']));
    expect(new Set(other.map(event => 'requestId' in event.kind ? event.kind.requestId : undefined)).size).toBe(1);
    const secondEvents = recovered.filter(event => event.sessionId === session.id);
    const requested = (events: EventEnvelope[], turnId?: string) =>
      events.find(event => event.kind.type === 'attention_requested' && event.turnId === turnId);
    const direct = requested(firstEvents, '1');
    const idle = requested(firstEvents);
    expect(direct).toBeDefined();
    expect(idle).toBeDefined();
    expect(direct?.eventId).not.toBe(idle?.eventId);
    expect(requested(secondEvents, '1')).toEqual(direct);
    expect(secondEvents).toHaveLength(3);
    expect(secondEvents.filter(event => event.kind.type === 'attention_resolved').map(event => event.turnId))
      .toEqual(expect.arrayContaining([undefined, '1']));
    expect(JSON.stringify([firstEvents, secondEvents])).not.toContain('私密');

    const fixture = join(directory, 'direct-events.json');
    await writeFile(fixture, JSON.stringify([firstEvents, secondEvents]));
    const result = spawnSync('cargo', ['test', '--locked', '-p', 'pet-core', '--test', 'harness_consumer', 'reloaded_direct_ask', '--', '--ignored'], {
      cwd: root, env: { ...process.env, PET_HARNESS_DIRECT_EVENTS: fixture }, encoding: 'utf8', stdio: 'pipe',
    });
    expect(result.status, result.stdout + result.stderr).toBe(0);
    expect(result.stdout).toContain('1 passed');
  } finally {
    await context.fiber.dispose();
    await rm(directory, { recursive: true, force: true });
  }
}, 120_000);
