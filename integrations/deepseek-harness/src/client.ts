import type { Context } from '@deepseek-ai/cordis';
import type {} from '@deepseek-ai/dsh-client-ui-workspace/client';
import type {} from '@deepseek-ai/dsh-api-session-controller/client';
import type { SessionId } from '@deepseek-ai/dsh-session/types';
import {
  consumeWebSessionLink,
  serveSessionLinks,
  type SessionLinkBridge,
  type WebSessionLocationWindow,
} from './session-links-client.js';

declare module '@deepseek-ai/dsh-api-session-controller/client' {
  interface SessionReferenceSourceMap { desktopPetLink: true }
}

/** 等待真实Client导航服务，Host自定义事件不会自动穿透Remote。 */
export const inject = ['sessions', 'uiWorkspace'];

/** 在兼容Electron补丁或本地Harness Web URL哈希入口接收目标，不创建会话或给模型发消息。 */
export function apply(ctx: Context): void {
  const open = async (id: string, signal: AbortSignal): Promise<void> => {
    const reference = ctx.sessions.retain(id as SessionId, { source: 'desktopPetLink', signal });
    try {
      await reference.ready;
      signal.throwIfAborted();
      ctx.uiWorkspace.openSession(reference.sessionId);
    } finally { reference.release(); }
  };
  const report = (): void => { console.warn('目标会话无法打开，请在Harness中检查会话后重试'); };
  const bridge = (globalThis as typeof globalThis & { dshDesktop?: { sessionLinks?: SessionLinkBridge } }).dshDesktop?.sessionLinks;
  if (bridge !== undefined) {
    ctx.effect(() => {
      const lifetime = new AbortController();
      const consumer = serveSessionLinks(bridge, open, lifetime.signal, report);
      return async () => { lifetime.abort(); consumer.dispose(); await consumer.settled(); };
    }, 'desktop-pet:session-links');
    return;
  }
  const win = globalThis as typeof globalThis & WebSessionLocationWindow;
  if (win.location === undefined) return;
  ctx.effect(() => {
    const lifetime = new AbortController();
    const consumer = consumeWebSessionLink(win, open, lifetime.signal, report);
    return async () => { lifetime.abort(); consumer.dispose(); await consumer.settled(); };
  }, 'desktop-pet:web-session-links');
}
