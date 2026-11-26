import { confirmChange } from './confirmation';
import { api, message, observeSnapshot } from '../api';
import { levelName, sourceName } from '../locales/zh-CN';
import type {
  ActivityRange,
  ActivityRangePreset,
  DeepSeekActivityStatus,
  DeepSeekModel,
  IntegrationStatus,
  PetSoundCue,
  Settings,
  SettingsPatch,
  SoundPack,
  Source,
} from '../types';

export const DEFAULT_SOUND_VOLUME = 0.7;

export const SOUND_PACK_CHOICES: ReadonlyArray<{ id: SoundPack; label: string }> = [
  { id: 'healing', label: '治愈萌音（灵动泛音与滑音，默认）' },
  { id: 'retro', label: '复古像素（8-bit 电子宠物）' },
  { id: 'minimal', label: '极简自然（水滴木琴风铃）' },
];

export const SOUND_VOLUME_PRESETS: ReadonlyArray<{ value: number; label: string }> = [
  { value: 0.3, label: '30% 轻柔' },
  { value: 0.7, label: '70% 适中' },
  { value: 1.0, label: '100% 饱满' },
];

export const SOUND_PREVIEW_ITEMS: ReadonlyArray<{ cue: PetSoundCue; label: string }> = [
  { cue: 'pet_head', label: '♪ 摸摸头' },
  { cue: 'feed', label: '♪ 投喂点心' },
  { cue: 'play', label: '♪ 开心玩耍' },
  { cue: 'dizzy', label: '♪ 转晕彩蛋' },
  { cue: 'alert_approval', label: '♪ 待审批' },
  { cue: 'alert_complete', label: '♪ 回合完成' },
  { cue: 'alert_failed', label: '♪ 失败提醒' },
];

export function normalizeSoundVolume(volume?: number): number {
  if (typeof volume !== 'number' || !Number.isFinite(volume)) return DEFAULT_SOUND_VOLUME;
  return Math.min(1, Math.max(0, Math.round(volume * 100) / 100));
}

export function formatSoundVolume(volume: number): string {
  return `${Math.round(normalizeSoundVolume(volume) * 100)}%`;
}

export const DEEPSEEK_MODEL_CHOICES: ReadonlyArray<{ id: DeepSeekModel; label: string }> = [
  { id: 'deepseek-chat', label: 'deepseek-chat（DeepSeek-V3，推荐）' },
  { id: 'deepseek-reasoner', label: 'deepseek-reasoner（深度思考）' },
];

export const BASE_WALK_SPEED_PX_PER_SEC = 175;
export const MIN_ACTIVITY_SPEED = 0.5;
export const MAX_ACTIVITY_SPEED = 2.0;
export const MIN_ACTIVITY_RANGE_SPAN = 0.1;
export const DEFAULT_BUBBLE_AUTO_CLOSE_SECONDS = 15;
export const MAX_BUBBLE_AUTO_CLOSE_SECONDS = 600;

export const BUBBLE_AUTO_CLOSE_OPTIONS: ReadonlyArray<{ value: number; label: string }> = [
  { value: 0, label: '不自动关闭' },
  { value: 5, label: '5 秒' },
  { value: 10, label: '10 秒' },
  { value: 15, label: '15 秒（默认）' },
  { value: 30, label: '30 秒' },
  { value: 60, label: '1 分钟' },
  { value: 180, label: '3 分钟' },
  { value: 300, label: '5 分钟' },
];

export function normalizeBubbleAutoCloseSeconds(seconds?: number): number {
  if (typeof seconds !== 'number' || !Number.isFinite(seconds)) return DEFAULT_BUBBLE_AUTO_CLOSE_SECONDS;
  return Math.min(MAX_BUBBLE_AUTO_CLOSE_SECONDS, Math.max(0, Math.round(seconds)));
}

export function formatBubbleAutoCloseSeconds(seconds: number): string {
  const normalized = normalizeBubbleAutoCloseSeconds(seconds);
  const preset = BUBBLE_AUTO_CLOSE_OPTIONS.find((item) => item.value === normalized);
  if (preset) return preset.label;
  if (normalized % 60 === 0) return `${normalized / 60} 分钟`;
  return `${normalized} 秒`;
}

export const ACTIVITY_RANGE_PRESETS: ReadonlyArray<{
  id: ActivityRangePreset;
  label: string;
  bounds?: readonly [number, number, number, number];
}> = [
  { id: 'full', label: '全屏工作区', bounds: [0, 1, 0, 1] },
  { id: 'bottom', label: '屏幕底部', bounds: [0, 1, 0.7, 1] },
  { id: 'lower_half', label: '屏幕下半区', bounds: [0, 1, 0.5, 1] },
  { id: 'left_edge', label: '左侧边缘', bounds: [0, 0.25, 0, 1] },
  { id: 'right_edge', label: '右侧边缘', bounds: [0.75, 1, 0, 1] },
  { id: 'custom', label: '自定义区域' },
];

export const ACTIVITY_SPEED_PRESETS: ReadonlyArray<{ value: number; label: string }> = [
  { value: 0.5, label: '0.5x 悠闲' },
  { value: 1.0, label: '1.0x 适中' },
  { value: 1.5, label: '1.5x 轻快' },
  { value: 2.0, label: '2.0x 飞奔' },
];

export function normalizeActivityRange(range?: ActivityRange): ActivityRange {
  const preset = range?.preset ?? 'full';
  const presetMeta = ACTIVITY_RANGE_PRESETS.find((item) => item.id === preset);
  if (presetMeta?.bounds) {
    return {
      preset,
      minX: presetMeta.bounds[0],
      maxX: presetMeta.bounds[1],
      minY: presetMeta.bounds[2],
      maxY: presetMeta.bounds[3],
    };
  }
  const clamp01 = (v: number, fallback: number) =>
    Number.isFinite(v) ? Math.min(1, Math.max(0, Math.round(v * 100) / 100)) : fallback;
  let minX = clamp01(range?.minX ?? 0, 0);
  let maxX = clamp01(range?.maxX ?? 1, 1);
  let minY = clamp01(range?.minY ?? 0, 0);
  let maxY = clamp01(range?.maxY ?? 1, 1);
  if (maxX - minX < MIN_ACTIVITY_RANGE_SPAN) {
    maxX = Math.min(1, Math.round((minX + MIN_ACTIVITY_RANGE_SPAN) * 100) / 100);
    minX = Math.max(0, Math.round((maxX - MIN_ACTIVITY_RANGE_SPAN) * 100) / 100);
  }
  if (maxY - minY < MIN_ACTIVITY_RANGE_SPAN) {
    maxY = Math.min(1, Math.round((minY + MIN_ACTIVITY_RANGE_SPAN) * 100) / 100);
    minY = Math.max(0, Math.round((maxY - MIN_ACTIVITY_RANGE_SPAN) * 100) / 100);
  }
  return { preset: 'custom', minX, maxX, minY, maxY };
}

export function normalizeActivitySpeed(speed?: number): number {
  if (typeof speed !== 'number' || !Number.isFinite(speed)) return 1.0;
  return Math.min(MAX_ACTIVITY_SPEED, Math.max(MIN_ACTIVITY_SPEED, Math.round(speed * 10) / 10));
}

export function formatActivityRangeSummary(range: ActivityRange): string {
  const normalized = normalizeActivityRange(range);
  const label = ACTIVITY_RANGE_PRESETS.find((item) => item.id === normalized.preset)?.label ?? '自定义区域';
  const pct = (v: number) => `${Math.round(v * 100)}%`;
  return `${label} · 横向 ${pct(normalized.minX)}–${pct(normalized.maxX)} · 纵向 ${pct(normalized.minY)}–${pct(normalized.maxY)}`;
}

export function formatActivitySpeed(speed: number): string {
  const normalized = normalizeActivitySpeed(speed);
  const pxPerSec = Math.round(normalized * BASE_WALK_SPEED_PX_PER_SEC);
  return `${normalized.toFixed(1)}x · ${pxPerSec} px/s`;
}

export type SettingsTab = 'general' | Source;

const SOURCES: readonly Source[] = ['codex', 'deepseek', 'antigravity'];

const TAB_META: Record<SettingsTab, { title: string; subtitle: string }> = {
  general: {
    title: '常规',
    subtitle: '桌宠外观、提醒声音、开机启动与自由活动配置',
  },
  codex: {
    title: sourceName.codex,
    subtitle: 'Codex CLI 与 Codex 桌面端的通知静音与 Hooks 接入',
  },
  deepseek: {
    title: sourceName.deepseek,
    subtitle: 'DeepSeek Harness 会话 SDK 插件接入与通知静音',
  },
  antigravity: {
    title: sourceName.antigravity,
    subtitle: 'Antigravity 桌面端 / IDE / CLI 接入与通知静音',
  },
};

const SVG_NS = 'http://www.w3.org/2000/svg';

function createNavIcon(doc: Document, tab: SettingsTab): SVGSVGElement {
  const svg = doc.createElementNS(SVG_NS, 'svg');
  svg.setAttribute('viewBox', '0 0 20 20');
  svg.setAttribute('fill', 'none');
  svg.setAttribute('stroke', 'currentColor');
  svg.setAttribute('stroke-width', '1.65');
  svg.setAttribute('stroke-linecap', 'round');
  svg.setAttribute('stroke-linejoin', 'round');
  svg.setAttribute('aria-hidden', 'true');
  svg.setAttribute('class', 'nav-icon');

  const add = (tag: 'path' | 'circle' | 'rect', attrs: Record<string, string>) => {
    const node = doc.createElementNS(SVG_NS, tag);
    for (const key of Object.keys(attrs)) node.setAttribute(key, attrs[key]);
    svg.append(node);
  };

  if (tab === 'general') {
    add('path', { d: 'M3.5 6h7.5M14.5 6h2M3.5 14h2M9 14h7.5' });
    add('circle', { cx: '12.75', cy: '6', r: '1.75' });
    add('circle', { cx: '7.25', cy: '14', r: '1.75' });
  } else if (tab === 'codex') {
    add('rect', { x: '2.75', y: '3.75', width: '14.5', height: '12.5', rx: '2.5' });
    add('path', { d: 'M6.25 8l2.5 2-2.5 2M10.5 12h3.25' });
  } else if (tab === 'deepseek') {
    add('path', { d: 'M10 3l6.25 3.5L10 10 3.75 6.5 10 3z' });
    add('path', { d: 'M3.75 10.25L10 13.75l6.25-3.5M3.75 13.75L10 17.25l6.25-3.5' });
  } else {
    add('circle', { cx: '10', cy: '10', r: '6.5' });
    add('path', { d: 'M7.5 12.5l1.6-4.1 4.1-1.6-1.6 4.1-4.1 1.6z' });
  }
  return svg;
}

export function formatActivityStatus(status: DeepSeekActivityStatus): string {
  if (!status.configured) return '当前状态：未配置 DeepSeek API Key（配置后桌宠即可自主活动）';
  if (!status.enabled) return `当前状态：已暂停自由活动 · 模型 ${status.model}`;
  if (status.lastError) return `当前状态：调用异常 · ${status.lastError}`;
  if (status.lastAction) {
    const thought = status.lastThought ? `（${status.lastThought}）` : '';
    return `当前状态：运行中 · 模型 ${status.model} · 最近活动：${status.lastAction}${thought}`;
  }
  return `当前状态：${status.running ? '运行中' : '待命中'} · 模型 ${status.model}`;
}

function activityStatusState(status: DeepSeekActivityStatus): string {
  if (!status.configured) return 'unconfigured';
  if (!status.enabled) return 'paused';
  if (status.lastError) return 'error';
  if (status.running || status.lastAction) return 'active';
  return 'idle';
}

type Action = 'install' | 'remove';

export function renderSource(
  doc: Document,
  status: IntegrationStatus,
  action: (source: Source, mode: Action) => void,
  controls: boolean,
): HTMLElement {
  const section = doc.createElement('article');
  section.className = 'source settings-card';
  section.dataset.installed = String(status.installed);

  const head = doc.createElement('div');
  head.className = 'source-header';

  const title = doc.createElement('h3');
  title.textContent = status.host;

  const meta = doc.createElement('p');
  meta.className = 'source-meta';

  const versionBadge = doc.createElement('span');
  versionBadge.className = 'badge badge-version';
  versionBadge.textContent = status.version || '版本未知';

  const stateBadge = doc.createElement('span');
  stateBadge.className = 'badge badge-state';
  stateBadge.dataset.installed = String(status.installed);
  stateBadge.textContent = status.installed ? '已登记自有配置' : '未确认接入';

  meta.append(versionBadge, stateBadge);
  head.append(title, meta);
  section.append(head);

  if (status.capabilities.length > 0) {
    const list = doc.createElement('dl');
    list.className = 'capability-list';
    for (const cap of status.capabilities) {
      const term = doc.createElement('dt');
      term.textContent = cap.name;
      const value = doc.createElement('dd');
      value.dataset.level = cap.status;
      const pill = doc.createElement('span');
      pill.className = 'level-pill';
      pill.dataset.level = cap.status;
      pill.textContent = levelName[cap.status];
      value.append(pill);
      list.append(term, value);
    }
    section.append(list);
  }

  if (status.problem) {
    const hint = doc.createElement('p');
    hint.className = 'source-note';
    hint.textContent = status.problem;
    section.append(hint);
  }

  if (status.installationId) {
    const id = doc.createElement('p');
    id.className = 'source-id';
    id.textContent = '安装 ID ' + status.installationId;
    section.append(id);
  }

  if (controls) {
    const targetWrap = doc.createElement('div');
    targetWrap.className = 'source-target';
    const targetLabel = doc.createElement('span');
    targetLabel.className = 'source-target-label';
    targetLabel.textContent = status.source === 'deepseek' ? 'Profile 目录' : '配置文件路径';
    const path = doc.createElement('code');
    path.textContent = status.target;
    targetWrap.append(targetLabel, path);
    section.append(targetWrap);

    const buttons = doc.createElement('div');
    buttons.className = 'actions';
    for (const mode of ['install', 'remove'] as const) {
      const b = doc.createElement('button');
      b.type = 'button';
      b.className =
        mode === 'install'
          ? 'btn btn-primary'
          : status.source === 'deepseek'
            ? 'btn btn-secondary'
            : 'btn btn-danger';
      b.textContent =
        status.source === 'deepseek'
          ? mode === 'install'
            ? '生成安装包'
            : '查看撤销步骤'
          : mode === 'install'
            ? '安装或更新 Hooks'
            : '撤销自有 Hooks';
      b.onclick = () => action(status.source, mode);
      buttons.append(b);
    }
    section.append(buttons);
  }

  return section;
}

export async function mountSettings(root: HTMLElement): Promise<() => void> {
  const doc = root.ownerDocument;
  const el = <K extends keyof HTMLElementTagNameMap>(tag: K, text = '', className = '') => {
    const n = doc.createElement(tag);
    if (text) n.textContent = text;
    if (className) n.className = className;
    return n;
  };

  const button = (text: string, run: () => void, variant = 'btn-secondary') => {
    const b = el('button', text, `btn ${variant}`);
    b.type = 'button';
    b.onclick = run;
    return b;
  };

  const option = (value: string, name: string) => {
    const o = el('option', name);
    o.value = value;
    return o;
  };

  const toggle = () => {
    const b = el('input', '', 'toggle');
    b.type = 'checkbox';
    return b;
  };

  const settingRow = (titleText: string, control: HTMLElement, description = '', interactiveGroup = false) => {
    const row = el(interactiveGroup ? 'div' : 'label', '', 'setting-row');
    const info = el('span', '', 'setting-row-info');
    info.append(el('span', titleText, 'setting-row-title'));
    if (description) info.append(el('span', description, 'setting-row-desc'));
    row.append(info, control);
    return row;
  };

  const settingCard = (headingText: string, ...children: HTMLElement[]) => {
    const group = el('section', '', 'settings-group');
    if (headingText) group.append(el('h3', headingText, 'settings-group-title'));
    const card = el('div', '', 'settings-card');
    card.append(...children);
    group.append(card);
    return group;
  };

  const shell = el('div', '', 'settings-shell');
  const sidebar = el('aside', '', 'settings-sidebar');
  const brand = el('div', '', 'settings-brand');
  brand.append(el('h1', '设置'), el('p', '桌宠与程序配置', 'settings-brand-sub'));

  const nav = el('nav', '', 'settings-nav');
  nav.setAttribute('role', 'tablist');
  nav.setAttribute('aria-label', '设置分类');

  const content = el('div', '', 'settings-content');
  const errors = el('p', '', 'banner banner-error');
  const report = el('p', '', 'banner banner-status');
  errors.setAttribute('role', 'alert');
  report.setAttribute('role', 'status');

  const form = el('fieldset', '', 'settings-fieldset');
  form.disabled = true;

  // Controls
  const fps = el('select'),
    pet = el('select'),
    scale = el('select'),
    soundPack = el('select'),
    bubbleAutoClose = el('select'),
    deepseekModel = el('select'),
    activityRangePreset = el('select');
  soundPack.name = 'soundPack';
  bubbleAutoClose.name = 'bubbleAutoCloseSeconds';
  deepseekModel.name = 'deepseekModel';
  activityRangePreset.name = 'activityRangePreset';

  fps.append(option('30', '30 FPS'), option('60', '60 FPS'));
  for (const v of [0.75, 1, 1.25, 1.5, 2]) scale.append(option(String(v), Math.round(100 * v) + '%'));
  for (const choice of SOUND_PACK_CHOICES) soundPack.append(option(choice.id, choice.label));
  for (const choice of BUBBLE_AUTO_CLOSE_OPTIONS)
    bubbleAutoClose.append(option(String(choice.value), choice.label));
  for (const choice of DEEPSEEK_MODEL_CHOICES) deepseekModel.append(option(choice.id, choice.label));
  for (const choice of ACTIVITY_RANGE_PRESETS)
    activityRangePreset.append(option(choice.id, choice.label));

  // Activity range visual preview & custom bounds
  const rangeBlock = el('div', '', 'activity-range-block');
  const rangePreviewWrap = el('div', '', 'activity-range-preview-wrap');
  const rangeMonitor = el('div', '', 'range-monitor');
  rangeMonitor.setAttribute('aria-hidden', 'true');
  const rangeMonitorBar = el('div', '', 'range-monitor-bar');
  const rangeScreen = el('div', '', 'range-screen');
  const rangeZone = el('div', '', 'range-zone');
  rangeZone.dataset.role = 'range-zone';
  const rangeZoneMarker = el('span', '', 'range-zone-marker');
  rangeZone.append(rangeZoneMarker);
  const rangeMonitorDock = el('div', '', 'range-monitor-dock');
  rangeScreen.append(rangeZone, rangeMonitorDock);
  rangeMonitor.append(rangeMonitorBar, rangeScreen);

  const rangePreviewMeta = el('div', '', 'range-preview-meta');
  const rangePreviewTitle = el('span', '桌面活动范围预览', 'range-preview-title');
  const rangePreviewSummary = el('span', '', 'range-preview-summary');
  rangePreviewSummary.dataset.role = 'range-summary';
  const rangePreviewCoverage = el('span', '', 'range-preview-coverage');
  rangePreviewCoverage.dataset.role = 'range-coverage';
  rangePreviewMeta.append(rangePreviewTitle, rangePreviewSummary, rangePreviewCoverage);
  rangePreviewWrap.append(rangeMonitor, rangePreviewMeta);

  const rangeCustomGrid = el('div', '', 'range-custom-grid');
  rangeCustomGrid.dataset.role = 'range-custom';
  rangeCustomGrid.hidden = true;

  const createRangeSlider = (name: string, labelText: string) => {
    const field = el('label', '', 'range-slider-field');
    const head = el('span', '', 'range-slider-head');
    const titleSpan = el('span', labelText, 'range-slider-label');
    const valueSpan = el('span', '0%', 'range-slider-value');
    head.append(titleSpan, valueSpan);
    const input = el('input', '', 'range-slider');
    input.type = 'range';
    input.name = name;
    input.min = '0';
    input.max = '100';
    input.step = '1';
    field.append(head, input);
    return { field, input, valueSpan };
  };

  const rangeMinX = createRangeSlider('activityRangeMinX', '左边界');
  const rangeMaxX = createRangeSlider('activityRangeMaxX', '右边界');
  const rangeMinY = createRangeSlider('activityRangeMinY', '上边界');
  const rangeMaxY = createRangeSlider('activityRangeMaxY', '下边界');
  rangeCustomGrid.append(rangeMinX.field, rangeMaxX.field, rangeMinY.field, rangeMaxY.field);
  rangeBlock.append(rangePreviewWrap, rangeCustomGrid);

  // Activity speed slider & presets
  const speedControl = el('div', '', 'speed-control');
  const speedSliderRow = el('div', '', 'speed-slider-row');
  const activitySpeed = el('input', '', 'range-slider');
  activitySpeed.type = 'range';
  activitySpeed.name = 'activitySpeed';
  activitySpeed.min = String(MIN_ACTIVITY_SPEED);
  activitySpeed.max = String(MAX_ACTIVITY_SPEED);
  activitySpeed.step = '0.1';
  const speedValueBadge = el('span', formatActivitySpeed(1.0), 'speed-value');
  speedValueBadge.dataset.role = 'speed-value';
  speedSliderRow.append(activitySpeed, speedValueBadge);

  const speedPresetsBar = el('div', '', 'speed-presets');
  const speedPresetButtons: Array<{ value: number; button: HTMLButtonElement }> = [];
  for (const preset of ACTIVITY_SPEED_PRESETS) {
    const chip = button(
      preset.label,
      () => {
        renderActivitySpeed(preset.value);
        update({ activitySpeed: preset.value });
      },
      'btn-chip',
    );
    chip.dataset.speed = String(preset.value);
    speedPresetButtons.push({ value: preset.value, button: chip });
    speedPresetsBar.append(chip);
  }
  speedControl.append(speedSliderRow, speedPresetsBar);

  const sound = toggle(),
    interactionSound = toggle(),
    ambientSound = toggle(),
    notifySubagents = toggle(),
    login = toggle(),
    deepseekEnabled = toggle(),
    muted: Record<Source, HTMLInputElement> = {
      codex: toggle(),
      deepseek: toggle(),
      antigravity: toggle(),
    };
  sound.name = 'sound';
  interactionSound.name = 'interactionSound';
  ambientSound.name = 'ambientSound';
  notifySubagents.name = 'notifySubagents';
  deepseekEnabled.name = 'deepseekEnabled';
  for (const s of SOURCES) muted[s].name = `muted-${s}`;

  // Sound volume slider & presets
  const volumeControl = el('div', '', 'speed-control');
  const volumeSliderRow = el('div', '', 'speed-slider-row');
  const soundVolume = el('input', '', 'range-slider');
  soundVolume.type = 'range';
  soundVolume.name = 'soundVolume';
  soundVolume.min = '0';
  soundVolume.max = '100';
  soundVolume.step = '1';
  const volumeValueBadge = el('span', formatSoundVolume(DEFAULT_SOUND_VOLUME), 'speed-value');
  volumeValueBadge.dataset.role = 'sound-volume-value';
  volumeSliderRow.append(soundVolume, volumeValueBadge);

  const volumePresetsBar = el('div', '', 'speed-presets');
  const volumePresetButtons: Array<{ value: number; button: HTMLButtonElement }> = [];
  for (const preset of SOUND_VOLUME_PRESETS) {
    const chip = button(
      preset.label,
      () => {
        renderSoundVolume(preset.value);
        update({ soundVolume: preset.value });
      },
      'btn-chip',
    );
    chip.dataset.volume = String(preset.value);
    volumePresetButtons.push({ value: preset.value, button: chip });
    volumePresetsBar.append(chip);
  }
  volumeControl.append(volumeSliderRow, volumePresetsBar);

  // Instant sound audition bar
  const soundPreviewBar = el('div', '', 'sound-preview-bar');
  const soundPreviewLabel = el('span', '试听当前宠物声线与主题音效', 'sound-preview-label');
  const soundPreviewChips = el('div', '', 'sound-preview-chips');
  for (const item of SOUND_PREVIEW_ITEMS) {
    const chip = button(
      item.label,
      () => {
        const rawVol = normalizeSoundVolume(Number(soundVolume.value) / 100);
        void Promise.resolve(
          api.previewPetSound({
            cue: item.cue,
            soundPack: ((soundPack.value as SoundPack) || current?.soundPack || 'healing'),
            soundVolume: rawVol,
            petId: pet.value || current?.petId,
          }),
        )
          .then(() => {
            if (rawVol <= 0.005) {
              showReport('当前总音量为 0%，已按 30% 临时音量试听');
            }
          })
          .catch(showError);
      },
      'btn-chip',
    );
    chip.dataset.previewCue = item.cue;
    soundPreviewChips.append(chip);
  }
  soundPreviewBar.append(soundPreviewLabel, soundPreviewChips);

  const deepseekApiKey = el('input');
  deepseekApiKey.type = 'password';
  deepseekApiKey.name = 'deepseekApiKey';
  deepseekApiKey.autocomplete = 'off';
  deepseekApiKey.spellcheck = false;
  deepseekApiKey.placeholder = '输入 sk-...';

  let keyVisible = false;
  const toggleKeyButton = button(
    '显示',
    () => {
      keyVisible = !keyVisible;
      deepseekApiKey.type = keyVisible ? 'text' : 'password';
      toggleKeyButton.textContent = keyVisible ? '隐藏' : '显示';
    },
    'btn-ghost btn-compact',
  );
  const apiKeyGroup = el('div', '', 'input-group');
  apiKeyGroup.append(deepseekApiKey, toggleKeyButton);

  const activityStatus = el('p', '当前状态：正在读取…', 'activity-status');
  activityStatus.dataset.state = 'idle';

  let current: Settings | undefined;
  let latestStatuses: IntegrationStatus[] = [];
  let saving = false,
    stopped = false;
  let clearKeyButton!: HTMLButtonElement;
  let triggerActivityButton!: HTMLButtonElement;

  const sourceContainers: Record<Source, HTMLElement> = {
    codex: el('div', '', 'source-list'),
    deepseek: el('div', '', 'source-list'),
    antigravity: el('div', '', 'source-list'),
  };
  const navStatusDots: Partial<Record<Source, HTMLSpanElement>> = {};

  function syncSidebarBadges() {
    for (const s of SOURCES) {
      const dot = navStatusDots[s];
      if (!dot) continue;
      const isMuted = current?.mutedSources.includes(s) ?? false;
      const isInstalled = latestStatuses.some((item) => item.source === s && item.installed);
      if (isMuted) {
        dot.dataset.status = 'muted';
        dot.title = isInstalled ? '已接入（已静音）' : '已静音';
      } else if (isInstalled) {
        dot.dataset.status = 'connected';
        dot.title = '已接入';
      } else {
        dot.dataset.status = 'disconnected';
        dot.title = '未接入';
      }
    }
  }

  function syncDeepSeekControls() {
    const configured = Boolean(current?.deepseekApiKeyConfigured);
    const enabled = current?.deepseekEnabled ?? true;
    clearKeyButton.disabled = saving || !configured;
    triggerActivityButton.disabled = saving || !configured || !enabled;
  }

  function renderActivityStatus(status: DeepSeekActivityStatus) {
    if (stopped) return;
    activityStatus.textContent = formatActivityStatus(status);
    activityStatus.dataset.state = activityStatusState(status);
  }

  function setSelectValue(select: HTMLSelectElement, value: string) {
    let matched: HTMLOptionElement | undefined;
    for (const opt of Array.from(select.options)) {
      if (opt.value === value) matched = opt;
      else opt.selected = false;
    }
    if (matched) matched.selected = true;
    try {
      select.value = value;
    } catch {
      // 部分轻量 DOM 测试环境（如 linkedom）的 select.value 仅提供 getter
    }
  }

  let draftRange: ActivityRange = normalizeActivityRange();

  function renderActivityRange(range: ActivityRange) {
    const normalized = normalizeActivityRange(range);
    draftRange = normalized;
    setSelectValue(activityRangePreset, normalized.preset);
    const leftPct = Math.round(normalized.minX * 100);
    const rightPct = Math.round(normalized.maxX * 100);
    const topPct = Math.round(normalized.minY * 100);
    const bottomPct = Math.round(normalized.maxY * 100);
    const widthPct = Math.max(0, rightPct - leftPct);
    const heightPct = Math.max(0, bottomPct - topPct);

    rangeZone.dataset.preset = normalized.preset;
    rangeZone.style.left = `${leftPct}%`;
    rangeZone.style.top = `${topPct}%`;
    rangeZone.style.width = `${widthPct}%`;
    rangeZone.style.height = `${heightPct}%`;

    rangePreviewSummary.textContent = formatActivityRangeSummary(normalized);
    const areaPct = Math.round((widthPct * heightPct) / 100);
    rangePreviewCoverage.textContent = `覆盖约 ${areaPct}% 桌面工作区`;

    rangeCustomGrid.hidden = normalized.preset !== 'custom';
    rangeMinX.input.value = String(leftPct);
    rangeMinX.valueSpan.textContent = `${leftPct}%`;
    rangeMaxX.input.value = String(rightPct);
    rangeMaxX.valueSpan.textContent = `${rightPct}%`;
    rangeMinY.input.value = String(topPct);
    rangeMinY.valueSpan.textContent = `${topPct}%`;
    rangeMaxY.input.value = String(bottomPct);
    rangeMaxY.valueSpan.textContent = `${bottomPct}%`;
  }

  function renderActivitySpeed(speed: number) {
    const normalized = normalizeActivitySpeed(speed);
    activitySpeed.value = normalized.toFixed(1);
    speedValueBadge.textContent = formatActivitySpeed(normalized);
    for (const item of speedPresetButtons) {
      const active = Math.abs(item.value - normalized) < 1e-6;
      item.button.classList.toggle('is-active', active);
      item.button.setAttribute('aria-pressed', String(active));
    }
  }

  function renderSoundVolume(volume: number) {
    const normalized = normalizeSoundVolume(volume);
    soundVolume.value = String(Math.round(normalized * 100));
    volumeValueBadge.textContent = formatSoundVolume(normalized);
    for (const item of volumePresetButtons) {
      const active = Math.abs(item.value - normalized) < 1e-6;
      item.button.classList.toggle('is-active', active);
      item.button.setAttribute('aria-pressed', String(active));
    }
  }

  function render(value: Settings) {
    if (stopped || (current && current.revision > value.revision)) return;
    current = value;
    setSelectValue(fps, String(value.fps));
    setSelectValue(pet, value.petId);
    if (!Array.from(scale.options).some((o) => Number(o.value) === value.placement.scale))
      scale.append(option(String(value.placement.scale), Math.round(value.placement.scale * 100) + '%'));
    setSelectValue(scale, String(value.placement.scale));
    deepseekEnabled.checked = value.deepseekEnabled ?? true;
    const model = value.deepseekModel ?? 'deepseek-chat';
    if (!Array.from(deepseekModel.options).some((o) => o.value === model))
      deepseekModel.append(option(model, model));
    setSelectValue(deepseekModel, model);
    deepseekApiKey.placeholder = value.deepseekApiKeyConfigured
      ? `已保存（${value.deepseekApiKeyHint || '••••'}），输入新 Key 可更新`
      : '输入 sk-...';
    renderActivityRange(value.activityRange ?? normalizeActivityRange());
    renderActivitySpeed(value.activitySpeed ?? 1.0);
    const autoCloseSeconds = normalizeBubbleAutoCloseSeconds(value.bubbleAutoCloseSeconds);
    if (!Array.from(bubbleAutoClose.options).some((o) => Number(o.value) === autoCloseSeconds))
      bubbleAutoClose.append(option(String(autoCloseSeconds), formatBubbleAutoCloseSeconds(autoCloseSeconds)));
    setSelectValue(bubbleAutoClose, String(autoCloseSeconds));
    sound.checked = value.sound;
    interactionSound.checked = value.interactionSound ?? true;
    ambientSound.checked = value.ambientSound ?? true;
    notifySubagents.checked = value.notifySubagents ?? true;
    const pack = value.soundPack ?? 'healing';
    setSelectValue(soundPack, pack);
    renderSoundVolume(value.soundVolume ?? DEFAULT_SOUND_VOLUME);
    login.checked = value.autostart;
    for (const s of SOURCES) muted[s].checked = value.mutedSources.includes(s);
    form.disabled = saving;
    syncDeepSeekControls();
    syncSidebarBadges();
  }

  function showError(e: unknown) {
    if (!stopped) {
      errors.textContent = message(e);
      errors.scrollIntoView?.({ block: 'nearest' });
    }
  }

  function showReport(text: string) {
    if (!stopped) {
      report.textContent = text;
      report.scrollIntoView?.({ block: 'nearest' });
    }
  }

  function setSourceButtonsDisabled(disabled: boolean) {
    for (const s of SOURCES) {
      sourceContainers[s].querySelectorAll('button').forEach((b) => {
        b.disabled = disabled;
      });
    }
  }

  async function work(action: () => Promise<void>) {
    if (saving || stopped) return;
    saving = true;
    form.disabled = true;
    setSourceButtonsDisabled(true);
    errors.textContent = '';
    report.textContent = '';
    try {
      await action();
    } catch (e) {
      showError(e);
    } finally {
      saving = false;
      setSourceButtonsDisabled(false);
      if (current) render(current);
      else form.disabled = false;
    }
  }

  const update = (patch: SettingsPatch) => {
    void work(async () => render(await api.update(patch)));
  };

  fps.onchange = () => update({ fps: Number(fps.value) as 30 | 60 });
  pet.onchange = () => update({ petId: pet.value });
  scale.onchange = () => {
    if (current) update({ placement: { scale: Number(scale.value) } });
  };
  deepseekEnabled.onchange = () => update({ deepseekEnabled: deepseekEnabled.checked });
  deepseekModel.onchange = () => update({ deepseekModel: deepseekModel.value as DeepSeekModel });
  activityRangePreset.onchange = () => {
    const preset = activityRangePreset.value as ActivityRangePreset;
    const nextRange =
      preset === 'custom'
        ? normalizeActivityRange({ ...draftRange, preset: 'custom' })
        : normalizeActivityRange({ ...draftRange, preset });
    renderActivityRange(nextRange);
    update({ activityRange: nextRange });
  };

  const applyCustomBound = (axis: 'minX' | 'maxX' | 'minY' | 'maxY', rawPct: number, commit: boolean) => {
    const minSpanPct = Math.round(MIN_ACTIVITY_RANGE_SPAN * 100);
    let minX = Math.round(draftRange.minX * 100);
    let maxX = Math.round(draftRange.maxX * 100);
    let minY = Math.round(draftRange.minY * 100);
    let maxY = Math.round(draftRange.maxY * 100);
    const clamped = Math.min(100, Math.max(0, Math.round(rawPct)));
    if (axis === 'minX') minX = Math.min(clamped, maxX - minSpanPct);
    else if (axis === 'maxX') maxX = Math.max(clamped, minX + minSpanPct);
    else if (axis === 'minY') minY = Math.min(clamped, maxY - minSpanPct);
    else maxY = Math.max(clamped, minY + minSpanPct);
    const nextRange: ActivityRange = {
      preset: 'custom',
      minX: minX / 100,
      maxX: maxX / 100,
      minY: minY / 100,
      maxY: maxY / 100,
    };
    renderActivityRange(nextRange);
    if (commit) update({ activityRange: nextRange });
  };

  const bindCustomSlider = (input: HTMLInputElement, axis: 'minX' | 'maxX' | 'minY' | 'maxY') => {
    input.oninput = () => applyCustomBound(axis, Number(input.value), false);
    input.onchange = () => applyCustomBound(axis, Number(input.value), true);
  };
  bindCustomSlider(rangeMinX.input, 'minX');
  bindCustomSlider(rangeMaxX.input, 'maxX');
  bindCustomSlider(rangeMinY.input, 'minY');
  bindCustomSlider(rangeMaxY.input, 'maxY');

  activitySpeed.oninput = () => renderActivitySpeed(Number(activitySpeed.value));
  activitySpeed.onchange = () => {
    const speed = normalizeActivitySpeed(Number(activitySpeed.value));
    renderActivitySpeed(speed);
    update({ activitySpeed: speed });
  };

  sound.onchange = () => update({ sound: sound.checked });
  interactionSound.onchange = () => update({ interactionSound: interactionSound.checked });
  ambientSound.onchange = () => update({ ambientSound: ambientSound.checked });
  soundPack.onchange = () => update({ soundPack: soundPack.value as SoundPack });
  soundVolume.oninput = () => renderSoundVolume(Number(soundVolume.value) / 100);
  soundVolume.onchange = () => {
    const vol = normalizeSoundVolume(Number(soundVolume.value) / 100);
    renderSoundVolume(vol);
    update({ soundVolume: vol });
  };
  bubbleAutoClose.onchange = () =>
    update({ bubbleAutoCloseSeconds: normalizeBubbleAutoCloseSeconds(Number(bubbleAutoClose.value)) });
  notifySubagents.onchange = () => update({ notifySubagents: notifySubagents.checked });
  login.onchange = () => update({ autostart: login.checked });
  for (const s of SOURCES)
    muted[s].onchange = () =>
      update({ mutedSources: SOURCES.filter((key) => muted[key].checked) });

  const saveApiKey = () => {
    const trimmed = deepseekApiKey.value.trim();
    if (!trimmed) {
      errors.textContent = '请输入 DeepSeek API Key，如需删除已保存的 Key 请点击「清除 Key」';
      return;
    }
    void work(async () => {
      const next = await api.update({
        deepseekApiKey: trimmed,
        deepseekModel: deepseekModel.value as DeepSeekModel,
        deepseekEnabled: deepseekEnabled.checked,
      });
      deepseekApiKey.value = '';
      render(next);
      showReport('已保存 DeepSeek API Key（仅用于桌宠自由活动）');
      void api.deepseekActivityStatus().then(renderActivityStatus).catch(() => {});
    });
  };

  deepseekApiKey.onkeydown = (event) => {
    if (event.key === 'Enter') {
      event.preventDefault();
      saveApiKey();
    }
  };

  const saveKeyButton = button('保存 API Key', saveApiKey, 'btn-primary');
  clearKeyButton = button(
    '清除 Key',
    () => {
      void work(async () => {
        const next = await api.update({ deepseekApiKey: '' });
        deepseekApiKey.value = '';
        render(next);
        showReport('已清除 DeepSeek API Key');
        void api.deepseekActivityStatus().then(renderActivityStatus).catch(() => {});
      });
    },
    'btn-danger btn-push-end',
  );
  const testKeyButton = button('测试连接', () => {
    const typed = deepseekApiKey.value.trim();
    const model = deepseekModel.value as DeepSeekModel;
    void work(async () => {
      const res = await api.testDeepSeekConnection(typed || undefined, model);
      showReport(`连接成功（${res.model} · ${res.latencyMs}ms）：${res.reply}`);
    });
  });
  triggerActivityButton = button('立即活动一次', () => {
    void work(async () => {
      await api.triggerPetActivity();
      showReport('已请求桌宠调用 DeepSeek 规划一次桌面活动');
    });
  });

  const autonomyActions = el('div', '', 'actions card-footer-actions');
  autonomyActions.append(saveKeyButton, testKeyButton, triggerActivityButton, clearKeyButton);

  async function listPets() {
    const choices = await api.pets();
    pet.replaceChildren(...choices.map((p) => option(p.id, p.name)));
    if (current) setSelectValue(pet, current.petId);
  }

  const importButton = button(
    '导入宠物包',
    () => {
      void work(async () => {
        const path = await api.chooseDirectory();
        if (!path) return;
        const imported = await api.importPet(path);
        await listPets();
        render(await api.update({ petId: imported.id }));
        showReport('已选用' + imported.name);
      });
    },
    'btn-secondary',
  );

  // Build General panel
  const createPaneHeader = (tab: SettingsTab, trailing?: HTMLElement) => {
    const header = el('header', '', 'pane-header');
    const titleGroup = el('div', '', 'pane-title-group');
    titleGroup.append(
      el('h2', TAB_META[tab].title, 'pane-title'),
      el('p', TAB_META[tab].subtitle, 'pane-subtitle'),
    );
    header.append(titleGroup);
    if (trailing) header.append(trailing);
    return header;
  };

  const petControl = el('div', '', 'control-inline');
  petControl.append(pet, importButton);

  const autonomyHint = el(
    'p',
    'DeepSeek API 仅用于桌宠在桌面上的自由活动（走动、跳跃、张望与短句互动），不用于会话通知或代码处理。',
    'card-intro',
  );

  const generalPanel = el('section', '', 'settings-panel');
  generalPanel.setAttribute('role', 'tabpanel');
  generalPanel.id = 'panel-general';
  generalPanel.setAttribute('aria-labelledby', 'tab-general');
  generalPanel.dataset.panel = 'general';
  generalPanel.append(
    createPaneHeader('general'),
    settingCard(
      '桌宠外观',
      settingRow('当前宠物', petControl, '选择桌面显示的宠物形象，或导入本地宠物包'),
      settingRow('动画帧率', fps, '更高帧率更顺滑，30 FPS 更省电'),
      settingRow('显示大小', scale, '调整桌宠在桌面上的缩放比例'),
    ),
    settingCard(
      '桌宠音效',
      settingRow('桌宠互动音效', interactionSound, '点击摸头、躲闪、投喂、玩耍与拖拽落地时播放宠物音效'),
      settingRow('自由活动轻音效', ambientSound, '桌宠自主蹦跳、招手或冒出想法时附带轻柔微音效'),
      settingRow('播放提醒声音', sound, '有待审批、待回答或回合结束时播放分级提示音'),
      settingRow('音效主题', soundPack, '切换音色风格，并随当前宠物形象自动微调声线'),
      settingRow('音效音量', volumeControl, '调节互动、自由活动与任务提醒的整体音量', true),
      soundPreviewBar,
    ),
    settingCard(
      '气泡与系统',
      settingRow(
        '任务气泡自动关闭',
        bubbleAutoClose,
        '任务提醒气泡在显示指定时长后自动收起（仍保留在通知面板中）',
      ),
      settingRow(
        '提示 Subagent 信息',
        notifySubagents,
        '关闭后不再弹出子代理（Subagent）的桌面提醒气泡、动作与提示音（仍保留在通知面板中）',
      ),
      settingRow('登录时启动桌宠', login, '系统登录后自动启动桌宠'),
    ),
    settingCard(
      '桌宠自由活动（DeepSeek API）',
      autonomyHint,
      activityStatus,
      settingRow('启用 DeepSeek 自由活动', deepseekEnabled, '空闲时让桌宠自主走动、张望与短句互动'),
      settingRow('DeepSeek API Key', apiKeyGroup, '仅保存在本机配置，用于自主活动规划'),
      settingRow('使用模型', deepseekModel, '推荐使用 deepseek-chat，响应更快'),
      settingRow('自由活动范围', activityRangePreset, '限制桌宠在桌面自由走动与蹦跳的活动区域'),
      rangeBlock,
      settingRow(
        '跑动速率',
        speedControl,
        '调节桌宠自主走动与蹦跳的移动速率（基准 175 px/s）',
        true,
      ),
      autonomyActions,
    ),
  );

  // Build per-program panels
  const programPanels: Record<Source, HTMLElement> = {} as Record<Source, HTMLElement>;
  for (const s of SOURCES) {
    const panel = el('section', '', 'settings-panel');
    panel.setAttribute('role', 'tabpanel');
    panel.id = `panel-${s}`;
    panel.setAttribute('aria-labelledby', `tab-${s}`);
    panel.dataset.panel = s;
    panel.hidden = true;

    const refreshButton = button(
      '刷新状态',
      () => {
        void listSources().catch(showError);
      },
      'btn-secondary',
    );

    const loading = el('p', '正在检测接入状态…', 'empty-state');
    sourceContainers[s].append(loading);

    const integrationGroup = el('section', '', 'settings-group');
    integrationGroup.append(el('h3', '宿主接入与能力', 'settings-group-title'), sourceContainers[s]);

    panel.append(
      createPaneHeader(s, refreshButton),
      settingCard(
        '通知提醒',
        settingRow(
          '静音 ' + sourceName[s],
          muted[s],
          `开启后不再弹出 ${sourceName[s]} 的桌面提醒气泡与提示音`,
        ),
      ),
      integrationGroup,
    );
    programPanels[s] = panel;
  }

  const allTabs: SettingsTab[] = ['general', ...SOURCES];
  const panels: Record<SettingsTab, HTMLElement> = {
    general: generalPanel,
    codex: programPanels.codex,
    deepseek: programPanels.deepseek,
    antigravity: programPanels.antigravity,
  };
  const tabButtons: Record<SettingsTab, HTMLButtonElement> = {} as Record<
    SettingsTab,
    HTMLButtonElement
  >;

  let activeTab: SettingsTab = 'general';

  function selectTab(next: SettingsTab, moveFocus = false) {
    if (activeTab !== next) {
      report.textContent = '';
    }
    activeTab = next;
    for (const tab of allTabs) {
      const selected = tab === next;
      const btn = tabButtons[tab];
      btn.setAttribute('aria-selected', String(selected));
      btn.tabIndex = selected ? 0 : -1;
      btn.classList.toggle('is-active', selected);
      panels[tab].hidden = !selected;
    }
    if (moveFocus) tabButtons[next].focus();
  }

  function createNavTab(tab: SettingsTab): HTMLButtonElement {
    const btn = el('button', '', 'nav-item');
    btn.type = 'button';
    btn.id = `tab-${tab}`;
    btn.setAttribute('role', 'tab');
    btn.setAttribute('aria-controls', `panel-${tab}`);
    btn.dataset.tab = tab;

    const icon = createNavIcon(doc, tab);
    const label = el('span', TAB_META[tab].title, 'nav-label');
    btn.append(icon, label);

    if (tab !== 'general') {
      const dot = el('span', '', 'nav-status-dot');
      dot.dataset.status = 'disconnected';
      dot.setAttribute('aria-hidden', 'true');
      navStatusDots[tab] = dot;
      btn.append(dot);
    }

    btn.onclick = () => selectTab(tab);
    btn.onkeydown = (event) => {
      const idx = allTabs.indexOf(tab);
      if (event.key === 'ArrowDown' || event.key === 'ArrowRight') {
        event.preventDefault();
        selectTab(allTabs[(idx + 1) % allTabs.length], true);
      } else if (event.key === 'ArrowUp' || event.key === 'ArrowLeft') {
        event.preventDefault();
        selectTab(allTabs[(idx - 1 + allTabs.length) % allTabs.length], true);
      }
    };

    tabButtons[tab] = btn;
    return btn;
  }

  nav.append(createNavTab('general'), el('div', '程序配置', 'settings-nav-group'));
  for (const s of SOURCES) nav.append(createNavTab(s));

  sidebar.append(brand, nav);
  form.append(generalPanel, programPanels.codex, programPanels.deepseek, programPanels.antigravity);
  content.append(errors, report, form);
  shell.append(sidebar, content);

  selectTab('general');
  renderActivityRange(draftRange);
  renderActivitySpeed(1.0);
  setSelectValue(bubbleAutoClose, String(DEFAULT_BUBBLE_AUTO_CLOSE_SECONDS));

  async function listSources() {
    const values = await api.integrations();
    if (stopped) return;
    latestStatuses = values;

    const grouped: Record<Source, IntegrationStatus[]> = {
      codex: [],
      deepseek: [],
      antigravity: [],
    };
    for (const item of values) {
      if (grouped[item.source]) grouped[item.source].push(item);
    }

    for (const s of SOURCES) {
      const items = grouped[s];
      if (items.length === 0) {
        const empty = el(
          'p',
          '暂未获取到接入状态，可点击右上角「刷新状态」重试。',
          'empty-state',
        );
        sourceContainers[s].replaceChildren(empty);
        continue;
      }
      sourceContainers[s].replaceChildren(
        ...items.map((status, index) =>
          renderSource(
            doc,
            status,
            (source, action) => {
              void work(async () => {
                const hookSource = source === 'codex' || source === 'antigravity';
                if (
                  hookSource &&
                  !(await confirmChange(
                    root,
                    (action === 'install'
                      ? '将备份配置并安装桌宠生命周期 Hooks'
                      : '将按安装清单撤销桌宠自有 Hooks') +
                      '。\n' +
                      status.target,
                    action === 'install' ? '确认安装' : '确认撤销',
                  ))
                )
                  return;
                const result = await api.integrate(source, action);
                const detail = [
                  hookSource ? (result.changed ? '自有配置已更新。' : '自有配置无需更改。') : '',
                  ...result.warnings,
                  result.bundlePath,
                  result.backupPath && '备份文件 ' + result.backupPath,
                ]
                  .filter(Boolean)
                  .join('\n');
                showReport(detail);
                await listSources();
              });
            },
            index === 0,
          ),
        ),
      );
    }
    syncSidebarBadges();
  }

  root.replaceChildren(shell);
  void listSources().catch(showError);
  void listPets().catch(showError);

  let stopActivityStatus: (() => void) | undefined;
  void api
    .deepseekActivityStatusEvents(renderActivityStatus)
    .then((unlisten) => {
      if (stopped) unlisten();
      else stopActivityStatus = unlisten;
    })
    .catch(() => {});
  void api
    .deepseekActivityStatus()
    .then(renderActivityStatus)
    .catch(() => {
      if (!stopped) {
        activityStatus.textContent = '当前状态：暂未获取';
        activityStatus.dataset.state = 'idle';
      }
    });

  const stop = await observeSnapshot(api.settingsEvents, api.settings, render, (e) => {
    showError(e);
    errors.append(
      button(
        '重试读取设置',
        () => {
          void api.settings().then(render).catch(showError);
        },
        'btn-secondary btn-compact',
      ),
    );
  });

  return () => {
    stopped = true;
    stopActivityStatus?.();
    stop();
  };
}
