import { spawn } from 'node:child_process';

/** 构建只承载一个会话ID的深链；不接受空白ID。 */
export function harnessSessionUrl(sessionId: string): string {
  if (!sessionId.trim()) throw new Error('会话标识不能为空');
  return `dsh://session/${encodeURIComponent(sessionId)}`;
}

/** 只有调用者确认安装兼容补丁后才提供启动器，成功仅表示已提交请求。 */
export interface HarnessSessionLauncher { request(url: string): Promise<void> }

/** 使用系统URL处理器；不经shell解释会话ID。 */
export function systemHarnessLauncher(): HarnessSessionLauncher {
  return { request: url => new Promise((resolve, reject) => {
    const command = process.platform === 'darwin' ? 'open' : process.platform === 'win32' ? 'rundll32.exe' : undefined;
    if (command === undefined) { reject(new Error('当前平台没有受支持的会话入口')); return; }
    const args = process.platform === 'win32' ? ['url.dll,FileProtocolHandler', url] : [url];
    const child = spawn(command, args, { stdio: 'ignore', windowsHide: true });
    child.once('error', reject);
    child.once('exit', code => code === 0 ? resolve() : reject(new Error('系统未接受会话入口请求')));
  }) };
}

/** 请求已安装的精确会话入口；不把应用唤起或请求接受等同于实际定位。 */
export async function openHarnessSession(sessionId: string, launcher?: HarnessSessionLauncher): Promise<{status:'requested'|'unavailable';reason?:string}> {
  if (launcher === undefined) return { status:'unavailable', reason:'尚未配置已验证的 Harness 会话入口' };
  try { await launcher.request(harnessSessionUrl(sessionId)); return { status:'requested' }; }
  catch { return { status:'unavailable', reason:'会话入口请求失败，请检查宿主补丁和系统协议注册' }; }
}
