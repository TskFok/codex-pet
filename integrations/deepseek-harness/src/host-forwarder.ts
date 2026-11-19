import {spawn} from 'node:child_process';
import type {EventEnvelope} from './events.js';

/** 有界串行调用原生转发器，由转发器负责耐久入队和本机投递。 */
export class HarnessForwarder {
  private tail:Promise<void>=Promise.resolve();
  private count=0;
  private bytes=0;
  private stopped=false;
  constructor(private readonly bridgePath:string,private readonly probeDir?:string){}
  /** 失败明确reject，由Host观察器隔离；既不抢答也不向模型写消息。 */
  send(event:EventEnvelope):Promise<void>{
    const data=JSON.stringify(event);
    const bytes=Buffer.byteLength(data);
    if(this.stopped||bytes>65536||this.count>=2000||this.bytes+bytes>10*1024*1024)return Promise.reject(new Error('桌宠转发未入队：已停止或超出队列限制'));
    this.count++;this.bytes+=bytes;
    const task=this.tail.then(()=>this.forward(data));
    this.tail=task.catch((_error:unknown)=>{ /* 单次失败由send调用者报告，后续事件继续尝试。 */ });
    return task.finally(()=>{this.count--;this.bytes-=bytes;});
  }
  private forward(data:string):Promise<void>{
    return new Promise((resolve,reject)=>{
      const args=['harness',...(this.probeDir===undefined?[]:['--probe-dir',this.probeDir])];
      const env=Object.fromEntries(Object.entries(process.env).filter(([name])=>!/(KEY|SECRET|TOKEN|PASSWORD)/i.test(name)));
      const child=spawn(this.bridgePath,args,{stdio:['pipe','ignore','ignore'],windowsHide:true,env,timeout:5000,killSignal:'SIGKILL'});
      let failed=false;
      child.once('error',()=>{failed=true;});
      child.once('close',(code,signal)=>!failed&&code===0&&signal===null?resolve():reject(new Error('桌宠转发器未接受事件；请检查投递配置')));
      child.stdin.on('error',()=>{failed=true;});
      child.stdin.end(data);
    });
  }
  /** 停止接收并等待全部已启动/排队子进程结束，不取消宿主请求。 */
  async dispose():Promise<void>{this.stopped=true;await this.tail;}
}
