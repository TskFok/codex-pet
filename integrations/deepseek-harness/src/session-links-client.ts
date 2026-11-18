/** 与兼容桌面补丁共享的窄IPC面；目标在明确确认前归Electron主进程所有。 */
export interface SessionLinkBridge {
  pending(): Promise<{id:number;sessionId:string}[]>;
  acknowledge(id:number,status:'opened'|'unavailable'):Promise<void>;
  subscribe(listener:()=>void):()=>void;
}

/** 串行消费入口；订阅先于读取，重载/撤销不提前确认正在打开的目标。 */
export function serveSessionLinks(bridge:SessionLinkBridge,open:(id:string,signal:AbortSignal)=>Promise<void>,signal:AbortSignal,report:()=>void):{dispose():void;settled():Promise<void>} {
  const lifetime = new AbortController();
  const combined = AbortSignal.any([signal, lifetime.signal]);
  let dirty = false;
  let task: Promise<void> | undefined;
  const wake = (): void => {
    dirty = true;
    if (task !== undefined || combined.aborted) return;
    task = (async () => {
      while (dirty && !combined.aborted) {
        dirty = false;
        const targets = await bridge.pending();
        for (const target of targets) {
          if (combined.aborted) return;
          let status: 'opened'|'unavailable' = 'opened';
          try { await open(target.sessionId, combined); }
          catch (_error) { status = 'unavailable'; }
          if (combined.aborted) return;
          await bridge.acknowledge(target.id, status);
          if (status === 'unavailable') report();
        }
      }
    })().catch((_error: unknown) => { if (!combined.aborted) report(); }).finally(() => { task = undefined; if (dirty && !combined.aborted) wake(); });
  };
  const unsubscribe = bridge.subscribe(wake);
  wake();
  return { dispose() { lifetime.abort(); unsubscribe(); }, async settled() { while (task !== undefined) await task; } };
}
