import { parseHTML } from 'linkedom';
import { expect, it, vi } from 'vitest';
import { api } from '../api';
import { mountSettings, renderSource } from './settings';
import type { IntegrationStatus, Settings } from '../types';

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

it('设置页支持填写 DeepSeek API Key、切换模型、测试连接与触发自由活动', async () => {
  const { document } = parseHTML('<html><body><main></main></body></html>');
  const root = document.querySelector('main')!;
  const initial: Settings = {
    revision: 1,
    fps: 30,
    sound: false,
    autostart: false,
    petId: 'yueying-cape-dancer',
    mutedSources: [],
    deepseekEnabled: true,
    deepseekModel: 'deepseek-chat',
    deepseekApiKeyConfigured: false,
    deepseekApiKeyHint: null,
    placement: { x: 100, y: 120, scale: 1, monitorId: null },
  };
  vi.spyOn(api, 'settings').mockResolvedValue(initial);
  vi.spyOn(api, 'settingsEvents').mockResolvedValue(() => {});
  vi.spyOn(api, 'pets').mockResolvedValue([{ id: 'yueying-cape-dancer', name: '月影' }]);
  vi.spyOn(api, 'integrations').mockResolvedValue([]);
  vi.spyOn(api, 'deepseekActivityStatus').mockResolvedValue({
    configured: false,
    enabled: true,
    running: false,
    model: 'deepseek-chat',
  });
  vi.spyOn(api, 'deepseekActivityStatusEvents').mockResolvedValue(() => {});
  let rev = 1;
  let state: Settings = { ...initial };
  const update = vi.spyOn(api, 'update').mockImplementation(async (patch) => {
    rev += 1;
    state = {
      ...state,
      revision: rev,
      deepseekModel: patch.deepseekModel ?? state.deepseekModel,
      deepseekEnabled: patch.deepseekEnabled ?? state.deepseekEnabled,
      deepseekApiKeyConfigured:
        patch.deepseekApiKey !== undefined ? patch.deepseekApiKey.length > 0 : state.deepseekApiKeyConfigured,
      deepseekApiKeyHint:
        patch.deepseekApiKey !== undefined
          ? patch.deepseekApiKey
            ? 'sk-t•••7890'
            : null
          : state.deepseekApiKeyHint,
    };
    return state;
  });
  const testConn = vi.spyOn(api, 'testDeepSeekConnection').mockResolvedValue({
    ok: true,
    model: 'deepseek-reasoner',
    latencyMs: 320,
    reply: '你好呀，准备去桌面逛逛！',
  });
  const trigger = vi.spyOn(api, 'triggerPetActivity').mockResolvedValue(undefined);

  const stop = await mountSettings(root);
  try {
    expect(root.querySelector('[role="alert"]')?.textContent).toBe('');
    expect(root.textContent).toContain('DeepSeek API 仅用于桌宠在桌面上的自由活动');
    const keyInput = root.querySelector<HTMLInputElement>('input[name="deepseekApiKey"]')!;
    const modelSelect = root.querySelector<HTMLSelectElement>('select[name="deepseekModel"]')!;
    expect(keyInput).toBeTruthy();
    expect(keyInput.type).toBe('password');
    expect(Array.from(modelSelect.options).map((o) => o.value)).toEqual([
      'deepseek-chat',
      'deepseek-reasoner',
    ]);

    const form = root.querySelector<HTMLFieldSetElement>('fieldset')!;
    modelSelect.options[0].selected = false;
    modelSelect.options[1].selected = true;
    modelSelect.onchange?.(new Event('change'));
    await vi.waitFor(() => expect(update).toHaveBeenCalledWith({ deepseekModel: 'deepseek-reasoner' }));
    await vi.waitFor(() => expect(form.disabled).toBe(false));

    keyInput.value = 'sk-test-key-7890';
    const saveButton = Array.from(root.querySelectorAll('button')).find((b) => b.textContent === '保存 API Key')!;
    saveButton.click();
    await vi.waitFor(() =>
      expect(update).toHaveBeenCalledWith({
        deepseekApiKey: 'sk-test-key-7890',
        deepseekModel: 'deepseek-reasoner',
        deepseekEnabled: true,
      }),
    );
    await vi.waitFor(() => expect(form.disabled).toBe(false));
    expect(keyInput.value).toBe('');
    expect(keyInput.placeholder).toContain('sk-t•••7890');

    const testButton = Array.from(root.querySelectorAll('button')).find((b) => b.textContent === '测试连接')!;
    testButton.click();
    await vi.waitFor(() => expect(testConn).toHaveBeenCalledWith(undefined, 'deepseek-reasoner'));
    await vi.waitFor(() => expect(form.disabled).toBe(false));
    expect(root.textContent).toContain('你好呀，准备去桌面逛逛！');

    const triggerButton = Array.from(root.querySelectorAll('button')).find((b) => b.textContent === '立即活动一次')!;
    triggerButton.click();
    await vi.waitFor(() => expect(trigger).toHaveBeenCalledTimes(1));
    await vi.waitFor(() => expect(form.disabled).toBe(false));

    const clearButton = Array.from(root.querySelectorAll('button')).find((b) => b.textContent === '清除 Key')!;
    clearButton.click();
    await vi.waitFor(() => expect(update).toHaveBeenCalledWith({ deepseekApiKey: '' }));
    await vi.waitFor(() => expect(form.disabled).toBe(false));
    expect(keyInput.placeholder).toBe('输入 sk-...');
  } finally {
    stop();
    vi.restoreAllMocks();
  }
});
