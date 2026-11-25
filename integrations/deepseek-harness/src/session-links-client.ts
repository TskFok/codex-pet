/** 与兼容桌面补丁共享的窄IPC面；目标在明确确认前归Electron主进程所有。 */
export interface SessionLinkBridge {
  pending(): Promise<{id:number;sessionId:string}[]>;
  acknowledge(id:number,status:'opened'|'unavailable'):Promise<void>;
  subscribe(listener:()=>void):()=>void;
}

export const WEB_SESSION_HASH_PREFIX = '#codex-pet-session=';

/** 解析本地 Web URL 哈希中的受限会话标识；非法输入一律忽略。 */
export function parseWebSessionHash(hash: string): string | undefined {
  if (!hash.startsWith(WEB_SESSION_HASH_PREFIX)) return undefined;
  const raw = hash.slice(WEB_SESSION_HASH_PREFIX.length);
  if (raw === '') return undefined;
  let id: string;
  try { id = decodeURIComponent(raw); } catch { return undefined; }
  if (
    id.trim() === ''
    || id.length > 1024
    || id === '.'
    || id === '..'
    || id.includes('://')
    || /[\u0000-\u001f\u007f-\u009f]/.test(id)
  ) return undefined;
  return id;
}

export interface WebSessionLocationWindow {
  location?: { readonly hash: string; readonly pathname: string; readonly search: string };
  history?: { replaceState(data: unknown, unused: string, url?: string | URL | null): void };
  addEventListener?(type: 'hashchange', listener: () => void): void;
  removeEventListener?(type: 'hashchange', listener: () => void): void;
}

/** 串行消费桌面补丁入口；订阅先于读取，重载/撤销不提前确认正在打开的目标。 */
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

/** 在 Harness Web 中消费 `#codex-pet-session=<编码ID>`，并在消费前清理 URL 哈希避免刷新重复触发。 */
export function consumeWebSessionLink(
  win: WebSessionLocationWindow,
  open: (id: string, signal: AbortSignal) => Promise<void>,
  signal: AbortSignal,
  report: () => void,
): { dispose(): void; settled(): Promise<void> } {
  const lifetime = new AbortController();
  const combined = AbortSignal.any([signal, lifetime.signal]);
  const queue: string[] = [];
  let task: Promise<void> | undefined;
  const drain = (): void => {
    if (task !== undefined || combined.aborted) return;
    task = (async () => {
      while (queue.length > 0 && !combined.aborted) {
        const id = queue.shift()!;
        try { await open(id, combined); }
        catch (_error) { if (!combined.aborted) report(); }
      }
    })().finally(() => { task = undefined; if (queue.length > 0 && !combined.aborted) drain(); });
  };
  const inspect = (): void => {
    if (combined.aborted || win.location === undefined) return;
    const id = parseWebSessionHash(win.location.hash);
    if (id === undefined) return;
    try { win.history?.replaceState(null, '', `${win.location.pathname}${win.location.search}`); } catch { /* ignore */ }
    queue.push(id);
    drain();
  };
  win.addEventListener?.('hashchange', inspect);
  inspect();
  return {
    dispose() {
      lifetime.abort();
      win.removeEventListener?.('hashchange', inspect);
    },
    async settled() {
      while (task !== undefined) await task;
    },
  };
}
