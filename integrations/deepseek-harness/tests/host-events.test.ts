import { Context } from '@deepseek-ai/cordis';
import SessionStore, { SessionId } from '@deepseek-ai/dsh-session';
import {ApprovalRequestId} from '@deepseek-ai/dsh-user-approval/types';
import type {} from '@deepseek-ai/dsh-user-questions/types';
import { expect, it } from 'vitest';
import { observeHost, type ObservationSink } from '../src/host-observer.js';
import type { HarnessObservation } from '../src/events.js';

async function setup(sink: ObservationSink) {
  const ctx = new Context();
  await ctx.plugin(SessionStore);
  observeHost(ctx, sink);
  const session = ctx.sessions.create(SessionId('s1'));
  session.append('turn/start', {turn:1});
  return { ctx, session };
}
it('question_listener_always_delegates，返回同一答案且只调用一次next', async () => {
  const seen: HarnessObservation[] = [];
  const {ctx} = await setup(observation => { seen.push(observation); });
  const answer = {answers:[{id:'q',selected:['A']}]};
  let calls = 0;
  const result = await ctx.waterfall('user-questions/request', {questions:[{id:'q',question:'不转发此原文'}], agent:{id:SessionId('s1')}}, async () => {calls++; return answer;});
  expect(result).toBe(answer); expect(calls).toBe(1);
  expect(seen.map(value => value.type)).toEqual(['turn/start','question/requested','question/resolved']);
  expect(seen[1]?.requestId).toBe(seen[2]?.requestId);
  expect(JSON.stringify(seen)).not.toContain('不转发此原文');
  await ctx.fiber.dispose();
});
it('waterfall异常原样传播，转发器同步或异步失败不影响宿主', async () => {
  for (const sink of [() => {throw Error('转发失败');}, async () => {throw Error('异步转发失败');}]) {
    const {ctx} = await setup(sink);
    const failure = Error('宿主失败'); let calls=0;
    await expect(ctx.waterfall('user-questions/request', {questions:[],agent:{id:SessionId('s1')}}, async () => {calls++;throw failure;})).rejects.toBe(failure);
    expect(calls).toBe(1);
    await ctx.fiber.dispose();
  }
});
it('cancelled_question_resolves_attention，即使下游尚未settle也只解除原请求', async () => {
  const seen: HarnessObservation[]=[];
  const {ctx}=await setup(observation=>{seen.push(observation);});
  const signal=new AbortController(); const gate=Promise.withResolvers<{answers:[]}>();
  const work=ctx.waterfall('user-questions/request',{questions:[],agent:{id:SessionId('s1')},signal:signal.signal},()=>gate.promise);
  signal.abort();
  expect(seen.map(value=>value.type)).toEqual(['turn/start','question/requested','question/resolved']);
  gate.resolve({answers:[]}); await work;
  expect(seen.filter(value=>value.type==='question/resolved')).toHaveLength(1);
  await ctx.fiber.dispose();
});
it('审批精确匹配、父子会话和两轮序号来自真实Session append',async()=>{
  const seen: HarnessObservation[]=[];
  const {ctx,session}=await setup(observation=>{seen.push(observation);});
  session.append('approval/asked',{id:ApprovalRequestId('a'),toolName:'tool'});
  session.append('approval/asked',{id:ApprovalRequestId('b'),toolName:'tool'});
  session.append('approval/decided',{id:ApprovalRequestId('a'),outcome:'cancelled'});
  session.append('turn/end',{turn:1,reason:{kind:'completed'}});
  session.append('turn/start',{turn:2});
  expect(seen.map(value=>value.seq)).toEqual([0,1,2,3,4,5]);
  expect(seen[3]?.requestId).toBe('a');
  expect(seen[5]?.turn).toBe(2);
  await ctx.fiber.dispose();
});

it('timed超时不解除，后续真实late reply仍使用原始出生turn',async()=>{
  const {default:SessionProjectionRegistry}=await import('@deepseek-ai/dsh-session-projection');
  const {createToolResultMessage,createUserMessage,ToolCallId}=await import('@deepseek-ai/dsh-llm');
  const {installQuestionBirths}=await import('../src/host-births.js');
  const seen:HarnessObservation[]=[];const ctx=new Context();
  await ctx.plugin(SessionStore); await ctx.plugin(SessionProjectionRegistry);
  const births=await installQuestionBirths(ctx);
  observeHost(ctx,observation=>{seen.push(observation);},undefined,births);
  const s=ctx.sessions.create(SessionId('s'));
  s.append('turn/start',{turn:1});
  s.append('tool/call',{turn:1,step:1,callId:ToolCallId('q'),name:'ask_user_question',arguments:'{}'});
  const failure=Object.assign(Error('timeout'),{code:'ASK_TIMED_OUT'});
  await expect(ctx.waterfall('user-questions/request',{agent:{id:s.id},questions:[],wait:{callId:ToolCallId('q'),timed:true}},async()=>{throw failure;})).rejects.toBe(failure);
  expect(seen.filter(e=>e.type==='question/resolved')).toHaveLength(0);
  expect(seen.find(e=>e.type==='question/requested')).toMatchObject({turn:1,lifetime:'until_resolved'});
  s.append('tool/result',{turn:1,step:1,message:createToolResultMessage({callId:ToolCallId('q'),content:[{type:'text',text:'{"pending":true}'}],isError:false})},{surfaceOp:'append'});
  s.append('turn/end',{turn:1,reason:{kind:'completed'}});
  s.append('turn/start',{turn:2});
  s.append('user/message',createUserMessage({source:{kind:'user-question-reply',callId:ToolCallId('q'),outcome:'answered'},content:[]}),{surfaceOp:'append'});
  expect(seen.filter(e=>e.type==='question/resolved')).toEqual([expect.objectContaining({turn:1,requestId:'question:q',seq:5})]);
  await ctx.fiber.dispose();
});
it('有会话但无活动turn的公开提问不伪造turn，直到真实回答解除',async()=>{
 const ctx=new Context();await ctx.plugin(SessionStore);const seen:HarnessObservation[]=[];observeHost(ctx,e=>{seen.push(e);});
 const s=ctx.sessions.create(SessionId('idle'));const gate=Promise.withResolvers<{answers:[]}>();
 const work=ctx.waterfall('user-questions/request',{agent:{id:s.id},questions:[]},()=>gate.promise);
 expect(seen[0]).toMatchObject({type:'question/requested',lifetime:'until_resolved'});
 expect(seen[0]?.turn).toBeUndefined();expect(seen[0]?.seq).toBeUndefined();
 s.append('turn/start',{turn:1});s.append('turn/end',{turn:1,reason:{kind:'completed'}});
 gate.resolve({answers:[]});await work;
 expect(seen.at(-1)?.turn).toBeUndefined();await ctx.fiber.dispose();
});
