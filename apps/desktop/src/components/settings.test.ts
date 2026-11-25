import { parseHTML } from 'linkedom';
import { expect, it, vi } from 'vitest';
import { renderSource } from './settings';
import type { IntegrationStatus } from '../types';

it('自有条目已写入时，仍独立呈现准确版本的能力级别，展开面板不会触发安装', () => {
  const { document } = parseHTML('<html><body></body></html>');
  const action = vi.fn();
  const status: IntegrationStatus = {
    source: 'codex',
    host: 'Codex CLI',
    version: '0.145.0',
    installationId: 'fixture',
    installed: true,
    target: '/isolated/hooks.json',
    capabilities: [
      { name: '回合开始', status: 'verified' },
      { name: '精确待处理同步', status: 'unsupported' },
      { name: '会话跳转', status: 'unverified' },
    ],
  };
  document.body.append(renderSource(document, status, action, true));
  for (const text of ['0.145.0', '已验证', '不支持', '未验证', '撤销'])
    expect(document.body.textContent).toContain(text);
  expect(action).not.toHaveBeenCalled();
});

it('Antigravity 接入卡片显示 Hooks 安装与撤销入口及分级能力', () => {
  const { document } = parseHTML('<html><body></body></html>');
  const action = vi.fn();
  const status: IntegrationStatus = {
    source: 'antigravity',
    host: 'Antigravity（桌面 / IDE / CLI）',
    version: null,
    installationId: 'ag-install',
    installed: true,
    target: '/isolated/.gemini/config/hooks.json',
    capabilities: [
      { name: '回合结束 / 中断 / 失败', status: 'unverified' },
      { name: '精确审批、提问与解除', status: 'unsupported' },
    ],
  };
  document.body.append(renderSource(document, status, action, true));
  for (const text of ['Antigravity（桌面 / IDE / CLI）', '安装或更新 Hooks', '撤销自有 Hooks', '不支持', '未验证'])
    expect(document.body.textContent).toContain(text);
  expect(action).not.toHaveBeenCalled();
});
