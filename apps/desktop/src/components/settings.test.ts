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
  vi.spyOn(api, 'pets').mockResolvedValue([{ id: 'yueying-cape-dancer', name: '夏乌拉' }]);
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

it('设置页将常规配置与各程序配置分标签展示，并在对应程序页独立控制静音与接入状态', async () => {
  const { document } = parseHTML('<html><body><main></main></body></html>');
  const root = document.querySelector('main')!;
  let state: Settings = {
    revision: 1,
    fps: 30,
    sound: true,
    autostart: false,
    petId: 'yueying-cape-dancer',
    mutedSources: ['deepseek'],
    deepseekEnabled: true,
    deepseekModel: 'deepseek-chat',
    deepseekApiKeyConfigured: false,
    deepseekApiKeyHint: null,
    placement: { x: 100, y: 120, scale: 1, monitorId: null },
  };
  const integrations: IntegrationStatus[] = [
    {
      source: 'codex',
      host: 'Codex CLI',
      version: '0.145.0',
      installed: true,
      target: '/tmp/.codex/hooks.json',
      capabilities: [{ name: '回合开始', status: 'verified' }],
    },
    {
      source: 'codex',
      host: 'Codex 桌面',
      version: null,
      installed: true,
      target: '/tmp/.codex/hooks.json',
      capabilities: [{ name: '回合开始', status: 'unverified' }],
    },
    {
      source: 'deepseek',
      host: 'DeepSeek Harness · 会话 SDK',
      version: '0.2.2-rc.1',
      installed: true,
      target: '/tmp/.dsh/profiles/desktop',
      capabilities: [{ name: 'SDK 连续观察审批与精确解除', status: 'verified' }],
    },
    {
      source: 'antigravity',
      host: 'Antigravity（桌面 / IDE / CLI）',
      version: null,
      installed: false,
      target: '/tmp/.gemini/config/hooks.json',
      capabilities: [{ name: '回合开始', status: 'unverified' }],
    },
  ];
  vi.spyOn(api, 'settings').mockResolvedValue(state);
  vi.spyOn(api, 'settingsEvents').mockResolvedValue(() => {});
  vi.spyOn(api, 'pets').mockResolvedValue([{ id: 'yueying-cape-dancer', name: '夏乌拉' }]);
  vi.spyOn(api, 'integrations').mockResolvedValue(integrations);
  vi.spyOn(api, 'deepseekActivityStatus').mockResolvedValue({
    configured: false,
    enabled: true,
    running: false,
    model: 'deepseek-chat',
  });
  vi.spyOn(api, 'deepseekActivityStatusEvents').mockResolvedValue(() => {});
  const update = vi.spyOn(api, 'update').mockImplementation(async (patch) => {
    state = {
      ...state,
      revision: state.revision + 1,
      mutedSources: patch.mutedSources ?? state.mutedSources,
    };
    return state;
  });

  const stop = await mountSettings(root);
  try {
    const tabs = Array.from(root.querySelectorAll<HTMLButtonElement>('[role="tab"]'));
    expect(tabs.map((t) => t.dataset.tab)).toEqual(['general', 'codex', 'deepseek', 'antigravity']);
    expect(tabs[0].textContent).toContain('常规');
    expect(tabs[0].getAttribute('aria-selected')).toBe('true');

    const generalPanel = root.querySelector<HTMLElement>('[data-panel="general"]')!;
    const codexPanel = root.querySelector<HTMLElement>('[data-panel="codex"]')!;
    const deepseekPanel = root.querySelector<HTMLElement>('[data-panel="deepseek"]')!;
    const antigravityPanel = root.querySelector<HTMLElement>('[data-panel="antigravity"]')!;

    expect(generalPanel.hidden).toBe(false);
    expect(codexPanel.hidden).toBe(true);
    expect(deepseekPanel.hidden).toBe(true);
    expect(antigravityPanel.hidden).toBe(true);

    // 常规面板包含宠物、通用提醒与 DeepSeek 自由活动，不包含各程序的静音开关或接入卡片
    expect(generalPanel.textContent).toContain('当前宠物');
    expect(generalPanel.textContent).toContain('播放提醒声音');
    expect(generalPanel.textContent).toContain('登录时启动桌宠');
    expect(generalPanel.textContent).toContain('桌宠自由活动（DeepSeek API）');
    expect(generalPanel.querySelector('input[name="muted-codex"]')).toBeNull();
    expect(generalPanel.querySelector('.source')).toBeNull();

    // 等待来源状态加载并检查侧边栏状态点
    await vi.waitFor(() => expect(codexPanel.querySelectorAll('.source').length).toBe(2));
    expect(deepseekPanel.querySelectorAll('.source').length).toBe(1);
    expect(antigravityPanel.querySelectorAll('.source').length).toBe(1);

    expect(tabs[1].querySelector<HTMLElement>('.nav-status-dot')?.dataset.status).toBe('connected');
    expect(tabs[2].querySelector<HTMLElement>('.nav-status-dot')?.dataset.status).toBe('muted');
    expect(tabs[3].querySelector<HTMLElement>('.nav-status-dot')?.dataset.status).toBe('disconnected');

    // 切换到 Codex 标签页，仅显示 Codex 的配置和接入卡片
    tabs[1].click();
    expect(generalPanel.hidden).toBe(true);
    expect(codexPanel.hidden).toBe(false);
    expect(codexPanel.textContent).toContain('静音 Codex');
    expect(codexPanel.textContent).toContain('Codex CLI');
    expect(codexPanel.textContent).toContain('Codex 桌面');
    expect(codexPanel.textContent).not.toContain('Antigravity');

    // 在 Codex 标签页切换静音开关
    const muteCodex = codexPanel.querySelector<HTMLInputElement>('input[name="muted-codex"]')!;
    expect(muteCodex.checked).toBe(false);
    muteCodex.checked = true;
    muteCodex.onchange?.(new Event('change'));
    await vi.waitFor(() => expect(update).toHaveBeenCalledWith({ mutedSources: ['codex', 'deepseek'] }));
    expect(tabs[1].querySelector<HTMLElement>('.nav-status-dot')?.dataset.status).toBe('muted');

    // 切换到 Antigravity 标签页
    tabs[3].click();
    expect(codexPanel.hidden).toBe(true);
    expect(antigravityPanel.hidden).toBe(false);
    expect(antigravityPanel.textContent).toContain('静音 Antigravity');
    expect(antigravityPanel.textContent).toContain('Antigravity（桌面 / IDE / CLI）');
  } finally {
    stop();
    vi.restoreAllMocks();
  }
});

it('设置页支持配置自由活动范围（预设与自定义预览）与跑动速率', async () => {
  const { document } = parseHTML('<html><body><main></main></body></html>');
  const root = document.querySelector('main')!;
  let state: Settings = {
    revision: 1,
    fps: 30,
    sound: false,
    autostart: false,
    petId: 'yueying-cape-dancer',
    mutedSources: [],
    deepseekEnabled: true,
    deepseekModel: 'deepseek-chat',
    deepseekApiKeyConfigured: true,
    deepseekApiKeyHint: 'sk-1•••9999',
    activityRange: { preset: 'full', minX: 0, maxX: 1, minY: 0, maxY: 1 },
    activitySpeed: 1.0,
    placement: { x: 100, y: 120, scale: 1, monitorId: null },
  };
  vi.spyOn(api, 'settings').mockResolvedValue(state);
  vi.spyOn(api, 'settingsEvents').mockResolvedValue(() => {});
  vi.spyOn(api, 'pets').mockResolvedValue([{ id: 'yueying-cape-dancer', name: '夏乌拉' }]);
  vi.spyOn(api, 'integrations').mockResolvedValue([]);
  vi.spyOn(api, 'deepseekActivityStatus').mockResolvedValue({
    configured: true,
    enabled: true,
    running: true,
    model: 'deepseek-chat',
  });
  vi.spyOn(api, 'deepseekActivityStatusEvents').mockResolvedValue(() => {});
  const update = vi.spyOn(api, 'update').mockImplementation(async (patch) => {
    state = {
      ...state,
      revision: state.revision + 1,
      activityRange: patch.activityRange ?? state.activityRange,
      activitySpeed: patch.activitySpeed ?? state.activitySpeed,
    };
    return state;
  });

  const stop = await mountSettings(root);
  try {
    const form = root.querySelector<HTMLFieldSetElement>('fieldset')!;
    const rangeSelect = root.querySelector<HTMLSelectElement>('select[name="activityRangePreset"]')!;
    const rangeZone = root.querySelector<HTMLElement>('[data-role="range-zone"]')!;
    const rangeSummary = root.querySelector<HTMLElement>('[data-role="range-summary"]')!;
    const rangeCoverage = root.querySelector<HTMLElement>('[data-role="range-coverage"]')!;
    const rangeCustom = root.querySelector<HTMLElement>('[data-role="range-custom"]')!;
    const speedSlider = root.querySelector<HTMLInputElement>('input[name="activitySpeed"]')!;
    const speedValue = root.querySelector<HTMLElement>('[data-role="speed-value"]')!;

    // 初始默认：全屏工作区 + 1.0x
    expect(rangeZone.style.left).toBe('0%');
    expect(rangeZone.style.top).toBe('0%');
    expect(rangeZone.style.width).toBe('100%');
    expect(rangeZone.style.height).toBe('100%');
    expect(rangeSummary.textContent).toContain('全屏工作区 · 横向 0%–100% · 纵向 0%–100%');
    expect(rangeCoverage.textContent).toContain('100%');
    expect(rangeCustom.hidden).toBe(true);
    expect(speedSlider.value).toBe('1.0');
    expect(speedValue.textContent).toBe('1.0x · 175 px/s');

    const chooseOption = (select: HTMLSelectElement, value: string) => {
      let target: HTMLOptionElement | undefined;
      for (const opt of Array.from(select.options)) {
        if (opt.value === value) target = opt;
        else opt.selected = false;
      }
      if (target) target.selected = true;
      select.onchange?.(new Event('change'));
    };

    // 切换预设到「屏幕底部」
    chooseOption(rangeSelect, 'bottom');
    await vi.waitFor(() =>
      expect(update).toHaveBeenCalledWith({
        activityRange: { preset: 'bottom', minX: 0, maxX: 1, minY: 0.7, maxY: 1 },
      }),
    );
    await vi.waitFor(() => expect(form.disabled).toBe(false));
    expect(rangeZone.style.top).toBe('70%');
    expect(rangeZone.style.height).toBe('30%');
    expect(rangeSummary.textContent).toContain('屏幕底部 · 横向 0%–100% · 纵向 70%–100%');
    expect(rangeCoverage.textContent).toContain('30%');
    expect(rangeCustom.hidden).toBe(true);

    // 切换预设到「自定义区域」，显示 4 个边界滑块并保留当前边界
    chooseOption(rangeSelect, 'custom');
    await vi.waitFor(() =>
      expect(update).toHaveBeenCalledWith({
        activityRange: { preset: 'custom', minX: 0, maxX: 1, minY: 0.7, maxY: 1 },
      }),
    );
    await vi.waitFor(() => expect(form.disabled).toBe(false));
    expect(rangeCustom.hidden).toBe(false);

    // 拖动自定义左边界仅即时更新预览，松手后持久化且保证最小跨度 10%
    const minXInput = root.querySelector<HTMLInputElement>('input[name="activityRangeMinX"]')!;
    const maxXInput = root.querySelector<HTMLInputElement>('input[name="activityRangeMaxX"]')!;
    maxXInput.value = '40';
    maxXInput.onchange?.(new Event('change'));
    await vi.waitFor(() =>
      expect(update).toHaveBeenCalledWith({
        activityRange: { preset: 'custom', minX: 0, maxX: 0.4, minY: 0.7, maxY: 1 },
      }),
    );
    await vi.waitFor(() => expect(form.disabled).toBe(false));

    const callsBeforeInput = update.mock.calls.length;
    minXInput.value = '35'; // 超过 maxX(40) - 10，应自动限制为 30
    minXInput.oninput?.(new Event('input'));
    expect(update.mock.calls.length).toBe(callsBeforeInput);
    expect(rangeZone.style.left).toBe('30%');
    expect(rangeZone.style.width).toBe('10%');

    minXInput.onchange?.(new Event('change'));
    await vi.waitFor(() =>
      expect(update).toHaveBeenCalledWith({
        activityRange: { preset: 'custom', minX: 0.3, maxX: 0.4, minY: 0.7, maxY: 1 },
      }),
    );
    await vi.waitFor(() => expect(form.disabled).toBe(false));

    // 拖动跑动速率滑块实时更新显示并在松手时保存
    speedSlider.value = '1.4';
    speedSlider.oninput?.(new Event('input'));
    expect(speedValue.textContent).toBe('1.4x · 245 px/s');
    speedSlider.onchange?.(new Event('change'));
    await vi.waitFor(() => expect(update).toHaveBeenCalledWith({ activitySpeed: 1.4 }));
    await vi.waitFor(() => expect(form.disabled).toBe(false));

    // 点击快捷速率档位按钮
    const fastChip = root.querySelector<HTMLButtonElement>('button[data-speed="2"]')!;
    fastChip.click();
    await vi.waitFor(() => expect(update).toHaveBeenCalledWith({ activitySpeed: 2 }));
    await vi.waitFor(() => expect(form.disabled).toBe(false));
    expect(speedValue.textContent).toBe('2.0x · 350 px/s');
    expect(fastChip.classList.contains('is-active')).toBe(true);
  } finally {
    stop();
    vi.restoreAllMocks();
  }
});

it('设置页支持配置任务气泡自动关闭时长与禁用自动关闭', async () => {
  const { document } = parseHTML('<html><body><main></main></body></html>');
  const root = document.querySelector('main')!;
  let state: Settings = {
    revision: 1,
    fps: 30,
    sound: false,
    autostart: false,
    petId: 'yueying-cape-dancer',
    mutedSources: [],
    bubbleAutoCloseSeconds: 15,
    placement: { x: 100, y: 120, scale: 1, monitorId: null },
  };
  let receiveSettings!: (value: Settings) => void;
  vi.spyOn(api, 'settings').mockResolvedValue(state);
  vi.spyOn(api, 'settingsEvents').mockImplementation(async (cb) => {
    receiveSettings = cb;
    return () => {};
  });
  vi.spyOn(api, 'pets').mockResolvedValue([{ id: 'yueying-cape-dancer', name: '夏乌拉' }]);
  vi.spyOn(api, 'integrations').mockResolvedValue([]);
  vi.spyOn(api, 'deepseekActivityStatus').mockResolvedValue({
    configured: false,
    enabled: true,
    running: false,
    model: 'deepseek-chat',
  });
  vi.spyOn(api, 'deepseekActivityStatusEvents').mockResolvedValue(() => {});
  const update = vi.spyOn(api, 'update').mockImplementation(async (patch) => {
    state = {
      ...state,
      revision: state.revision + 1,
      bubbleAutoCloseSeconds: patch.bubbleAutoCloseSeconds ?? state.bubbleAutoCloseSeconds,
    };
    return state;
  });

  const stop = await mountSettings(root);
  try {
    const form = root.querySelector<HTMLFieldSetElement>('fieldset')!;
    const select = root.querySelector<HTMLSelectElement>('select[name="bubbleAutoCloseSeconds"]')!;
    expect(select).toBeTruthy();
    expect(select.value).toBe('15');
    expect(root.textContent).toContain('任务气泡自动关闭');

    const chooseOption = (value: string) => {
      let target: HTMLOptionElement | undefined;
      for (const opt of Array.from(select.options)) {
        if (opt.value === value) target = opt;
        else opt.selected = false;
      }
      if (target) target.selected = true;
      select.onchange?.(new Event('change'));
    };

    chooseOption('0');
    await vi.waitFor(() => expect(update).toHaveBeenCalledWith({ bubbleAutoCloseSeconds: 0 }));
    await vi.waitFor(() => expect(form.disabled).toBe(false));
    expect(select.value).toBe('0');

    chooseOption('30');
    await vi.waitFor(() => expect(update).toHaveBeenCalledWith({ bubbleAutoCloseSeconds: 30 }));
    await vi.waitFor(() => expect(form.disabled).toBe(false));
    expect(select.value).toBe('30');

    // 兼容配置文件中非预设的自定义秒数
    receiveSettings({ ...state, revision: state.revision + 1, bubbleAutoCloseSeconds: 45 });
    expect(select.value).toBe('45');
    expect(Array.from(select.options).some((opt) => opt.value === '45' && opt.textContent === '45 秒')).toBe(true);
  } finally {
    stop();
    vi.restoreAllMocks();
  }
});
