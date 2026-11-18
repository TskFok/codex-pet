import type{Context}from'@deepseek-ai/cordis';
import type{}from'@deepseek-ai/dsh-session';
import type{}from'@deepseek-ai/dsh-session-projection';
import{isAbsolute}from'node:path';
import{z}from'zod';
import{installQuestionBirths}from'./host-births.js';
import{observeHost}from'./host-observer.js';
import{HarnessForwarder}from'./host-forwarder.js';
import{harnessIdentity,toPetEvents}from'./events.js';
export const Config=z.object({bridgePath:z.string().refine(isAbsolute,'bridgePath必须是绝对路径'),installationId:z.uuid({error:'installationId必须是持久UUID'}),probeDir:z.string().refine(isAbsolute,'probeDir必须是绝对路径').optional()}).strict();
export type PluginConfig=z.infer<typeof Config>;
export const inject=['sessions','sessionProjections'];
/** 加载正式Host观察器；身份由显式安装配置提供，不读写用户匿名身份或真实profile。 */
export function apply(ctx:Context,config:PluginConfig):void{
  const validated=Config.parse(config);
  ctx.effect(async()=>{
    const births=await installQuestionBirths(ctx);
    if(ctx.fiber.uid===null)return ()=>{};
    const forwarder=new HarnessForwarder(validated.bridgePath,validated.probeDir);
    observeHost(ctx,async observation=>{
      for(const event of toPetEvents(observation,harnessIdentity(observation,validated.installationId)))await forwarder.send(event);
    },births.turn,births);
    ctx.logger.info(validated.probeDir===undefined?'桌宠Host观察器已就绪；通知投递尚未配置':'桌宠Host观察器已就绪；当前只记录本地探测事件');
    return async()=>{await forwarder.dispose();};
  },'desktop-pet:host-observation');
}
