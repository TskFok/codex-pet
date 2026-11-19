import { writeSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

const [host, root, id, adapter, installationId] = process.argv.slice(2);
const packageModule = path => import(pathToFileURL(join(host, path, 'lib/index.js')).href);
const adapterModule = name => import(pathToFileURL(join(adapter, name + '.js')).href);
const [
  { Context }, { default: Sessions }, { default: Projections }, { default: Questions }, Persistence,
  { installQuestionBirths }, { observeHost }, { harnessIdentity, toPetEvents },
] = await Promise.all([
  packageModule('vendor/cordis'), packageModule('packages/core/session'),
  packageModule('packages/session/session-projection'), packageModule('packages/interaction/user-questions'),
  packageModule('packages/session/session-persistence-jsonl'),
  adapterModule('host-births'), adapterModule('host-observer'), adapterModule('events'),
]);

const ctx = new Context();
await ctx.plugin(Sessions);
await ctx.plugin(Projections);
await ctx.plugin(Questions);
await ctx.plugin(Persistence, { root, compression: 'none' });
const births = await installQuestionBirths(ctx);
const observations = [];
const observer = observeHost(ctx, value => observations.push(...toPetEvents(value, harnessIdentity(value, installationId))), births.turn, births);
await observer.ready;
const handle = await ctx.sessionPersistence.open(id, 'write');
const cold = await handle.read();
const proof = await handle.prepareObservationRecovery?.();
const session = ctx.sessions.prepare(id, {
  meta: handle.header, inheritedEventCount: handle.inheritedEventCount, seed: cold.events,
  eventState: cold.eventState, observationRecovery: proof,
});
await handle.append(session.snapshotEvents(cold.events.length));
ctx.sessions.enter(session);
ctx.sessions.announce(session);
session.append('turn/start', { turn: 2 });
session.append('turn/end', { turn: 2, reason: { kind: 'completed' } });
writeSync(1, JSON.stringify(observations));
process.kill(process.pid, 'SIGKILL');
