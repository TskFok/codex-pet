import { expect, it } from 'vitest';
import { harnessSessionUrl, openHarnessSession } from '../src/open-session.js';
it('会话ID编码为单一路径，拒绝空值', () => {
  expect(harnessSessionUrl('会话/a?#')).toBe('dsh://session/%E4%BC%9A%E8%AF%9D%2Fa%3F%23');
  expect(() => harnessSessionUrl(' ')).toThrow();
});
it('没有已验证入口时不把应用唤起报告为精确请求', async () => {
  expect(await openHarnessSession('s1')).toEqual({ status: 'unavailable', reason: '尚未配置已验证的 Harness 会话入口' });
});
