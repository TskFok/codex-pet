import type {Context} from '@deepseek-ai/cordis';
import type {Session, SessionEvent} from '@deepseek-ai/dsh-session';
import type {} from '@deepseek-ai/dsh-session-projection';
import type {} from '@deepseek-ai/dsh-user-questions/types';
import type {} from '@deepseek-ai/dsh-tool-executor-ptc/types';
import {z} from 'zod';

/** 请求的真实出生回合及日志位置；时间只供事件显示。 */
export interface QuestionBirth {turn:number|null;seq:number;time:number;timed:boolean}
interface ClosedQuestion extends QuestionBirth {requestId:string;callId:string;closedSeq:number}
interface BirthState {cut:number;openTurn:number|null;timedTool:boolean;active:Record<string,QuestionBirth>;lastClosed:ClosedQuestion|null}
/** 插件内部的领域读取面，不暴露Projection handle或重新扫描日志。 */
export interface QuestionBirths {
  turn(session:Session):number|undefined;
  birth(session:Session,callId:string):QuestionBirth|undefined;
  closed(session:Session,seq:number):({requestId:string;callId:string}&QuestionBirth)|undefined;
}
const birthSchema=z.object({turn:z.number().int().nonnegative().nullable(),seq:z.number().int().nonnegative(),time:z.number().int().nonnegative(),timed:z.boolean()});
const schema=z.object({cut:z.number().int().nonnegative(),openTurn:z.number().int().nonnegative().nullable(),timedTool:z.boolean(),active:z.record(z.string(),birthSchema),lastClosed:birthSchema.extend({requestId:z.string(),callId:z.string(),closedSeq:z.number().int().nonnegative()}).nullable()});
function pending(content:readonly {type:string;text?:string}[]):boolean {
  const text=content.find(block=>block.type==='text')?.text;
  if(text===undefined)return false;
  try {const value:unknown=JSON.parse(text);return typeof value==='object'&&value!==null&&'pending'in value&&value.pending===true;}
  catch(_error){ /* 失败工具的文本不是pending JSON。 */ return false;}
}
function birthOf(state:BirthState,id:string):QuestionBirth|undefined{return Object.hasOwn(state.active,id)?state.active[id]:undefined;}
function close(state:BirthState,id:string,event:SessionEvent):BirthState {
  const birth=birthOf(state,id); if(birth===undefined)return state;
  const active={...state.active}; delete active[id];
  return {...state,active,lastClosed:{...birth,requestId:`question:${id}`,callId:id,closedSeq:event.seq}};
}
function fold(state:BirthState,event:SessionEvent):BirthState {
  if(event.seq<state.cut)return state;
  switch(event.type){
    case 'turn/start':return {...state,openTurn:event.data.turn};
    case 'turn/end':return state.openTurn===event.data.turn?{...state,openTurn:null}:state;
    case 'request/header':
      return {...state,timedTool:event.data.header.tools?.some(tool => {
        const properties = tool.parameters.properties;
        return tool.name === 'ask_user_question' && typeof properties === 'object' && properties !== null && 'timeout' in properties;
      }) ?? false};
    case 'tool/call':
      return event.data.name==='ask_user_question'?{...state,active:{...state.active,[event.data.callId]:{turn:event.data.turn,seq:event.seq,time:event.time,timed:state.timedTool}}}:state;
    case 'tool/ptc-dispatch-start':
      return event.data.name==='ask_user_question'?{...state,active:{...state.active,[event.data.subCallId]:{turn:state.openTurn,seq:event.seq,time:event.time,timed:state.timedTool}}}:state;
    case 'tool/result':
      return pending(event.data.message.content)||event.data.error?.code==='TOOL_OUTCOME_UNKNOWN'?state:close(state,event.data.message.toolCallId,event);
    case 'tool/ptc-dispatch':
      if(event.data.name!=='ask_user_question')return state;
      if(!event.data.isError&&pending(event.data.content)){
        // 历史PTC日志可能只有settling事件，仍由其所在的真实turn/start确定回合。
        const birth = birthOf(state,event.data.subCallId) ?? {turn:state.openTurn,seq:event.seq,time:event.time};
        return {...state,active:{...state.active,[event.data.subCallId]:{...birth,timed:true}}};
      }
      return close(state,event.data.subCallId,event);
    case 'user/message':return event.data.source.kind==='user-question-reply'?close(state,event.data.source.callId,event):state;
    default:return state;
  }
}
/** 注册确定性Host-only投影；状态只保留尚未settle的ask调用和最后一个解除，随当前并发数增长。 */
export async function installQuestionBirths(ctx:Context):Promise<QuestionBirths>{
  const handle=await ctx.sessionProjections.register({key:'desktopPetQuestionBirths',stateVersion:2,stateSchema:schema,
    init:(_header,cut):BirthState=>({cut,openTurn:null,timedTool:false,active:{},lastClosed:null}),apply:fold});
  return {
    turn:session=>handle.stateOf(session).openTurn??undefined,
    birth:(session,id)=>birthOf(handle.stateOf(session),id),
    closed:(session,seq)=>{const result=handle.stateOf(session).lastClosed;return result?.closedSeq===seq?result:undefined;},
  };
}
