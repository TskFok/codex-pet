import { confirmChange } from './confirmation';
import { api, message, observeSnapshot } from '../api';
import { levelName, sourceName } from '../locales/zh-CN';
import type { IntegrationStatus, Settings, SettingsPatch, Source } from '../types';

type Action = 'install' | 'remove';
export function renderSource(
  doc: Document,
  status: IntegrationStatus,
  action: (source: Source, mode: Action) => void,
  controls: boolean,
): HTMLElement {
  const section = doc.createElement('article');
  section.className = 'source';
  const title = doc.createElement('h3');
  title.textContent = status.host;
  section.append(title);
  const meta = doc.createElement('p');
  meta.textContent = [status.version || '版本未知', status.installed ? '已登记自有配置' : '未确认接入'].join(
    ' · ',
  );
  section.append(meta);
  const list = doc.createElement('dl');
  for (const cap of status.capabilities) {
    const term = doc.createElement('dt');
    term.textContent = cap.name;
    const value = doc.createElement('dd');
    value.textContent = levelName[cap.status];
    value.dataset.level = cap.status;
    list.append(term, value);
  }
  section.append(list);
  if (status.problem) {
    const hint = doc.createElement('p');
    hint.textContent = status.problem;
    section.append(hint);
  }
  if (status.installationId) {
    const id = doc.createElement('p');
    id.textContent = '安装 ID ' + status.installationId;
    section.append(id);
  }
  if (controls) {
    const path = doc.createElement('code');
    path.textContent = status.target;
    section.append(path);
    const buttons = doc.createElement('div');
    buttons.className = 'actions';
    for (const mode of ['install', 'remove'] as const) {
      const b = doc.createElement('button');
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
  const el = <K extends keyof HTMLElementTagNameMap>(tag: K, text = '') => {
    const n = doc.createElement(tag);
    n.textContent = text;
    return n;
  };
  const title = el('h1', '设置'),
    form = el('fieldset'),
    sources = el('section'),
    report = el('p'),
    errors = el('p');
  report.setAttribute('role', 'status');
  errors.setAttribute('role', 'alert');
  const appearance = el('h2', '桌宠'),
    alerts = el('h2', '提醒');
  const row = (text: string, control: HTMLElement) => {
    const label = el('label');
    label.append(el('span', text), control);
    return label;
  };
  const button = (text: string, run: () => void) => {
    const b = el('button', text);
    b.onclick = run;
    return b;
  };
  const fps = el('select'),
    pet = el('select'),
    scale = el('select');
  const option = (value: string, name: string) => {
    const o = el('option', name);
    o.value = value;
    return o;
  };
  fps.append(option('30', '30 FPS'), option('60', '60 FPS'));
  for (const v of [0.75, 1, 1.25, 1.5, 2]) scale.append(option(String(v), Math.round(100 * v) + '%'));
  const checkbox = () => {
    const b = el('input');
    b.type = 'checkbox';
    return b;
  };
  const SOURCES = ['codex', 'deepseek', 'antigravity'] as const;
  const sound = checkbox(),
    login = checkbox(),
    muted = { codex: checkbox(), deepseek: checkbox(), antigravity: checkbox() };
  let current: Settings | undefined;
  let saving = false,
    stopped = false;
  function render(value: Settings) {
    if (stopped || (current && current.revision > value.revision)) return;
    current = value;
    fps.value = String(value.fps);
    pet.value = value.petId;
    if (!Array.from(scale.options).some((o) => Number(o.value) === value.placement.scale))
      scale.append(option(String(value.placement.scale), Math.round(value.placement.scale * 100) + '%'));
    scale.value = String(value.placement.scale);
    sound.checked = value.sound;
    login.checked = value.autostart;
    for (const s of SOURCES) muted[s].checked = value.mutedSources.includes(s);
    form.disabled = saving;
  }
  function showError(e: unknown) {
    if (!stopped) errors.textContent = message(e);
  }
  async function work(action: () => Promise<void>) {
    if (saving || stopped) return;
    saving = true;
    form.disabled = true;
    sources.querySelectorAll('button').forEach((b) => {
      b.disabled = true;
    });
    errors.textContent = '';
    report.textContent = '';
    try {
      await action();
    } catch (e) {
      showError(e);
    } finally {
      saving = false;
      sources.querySelectorAll('button').forEach((b) => {
        b.disabled = false;
      });
      if (current) render(current);
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
  sound.onchange = () => update({ sound: sound.checked });
  login.onchange = () => update({ autostart: login.checked });
  for (const s of SOURCES)
    muted[s].onchange = () =>
      update({ mutedSources: SOURCES.filter((key) => muted[key].checked) });
  async function listPets() {
    const choices = await api.pets();
    pet.replaceChildren(...choices.map((p) => option(p.id, p.name)));
    if (current) pet.value = current.petId;
  }
  const importButton = button('导入宠物包', () => {
    void work(async () => {
      const path = await api.chooseDirectory();
      if (!path) return;
      const imported = await api.importPet(path);
      await listPets();
      render(await api.update({ petId: imported.id }));
      report.textContent = '已选用' + imported.name;
    });
  });
  form.disabled = true;
  form.append(
    appearance,
    row('当前宠物', pet),
    importButton,
    row('动画帧率', fps),
    row('显示大小', scale),
    alerts,
    row('播放提醒声音', sound),
    ...SOURCES.map((s) => row('静音 ' + sourceName[s], muted[s])),
    row('登录时启动桌宠', login),
  );
  async function listSources() {
    const values = await api.integrations();
    if (stopped) return;
    const header = el('header');
    header.append(
      el('h2', '来源接入'),
      button('刷新状态', () => {
        void listSources().catch(showError);
      }),
    );
    const seen = new Set<Source>();
    sources.replaceChildren(
      header,
      ...values.map((s) => {
        const controls = !seen.has(s.source);
        seen.add(s.source);
        return renderSource(
          doc,
          s,
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
                    s.target,
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
              report.textContent = detail;
              report.scrollIntoView({ block: 'nearest' });
              await listSources();
            });
          },
          controls,
        );
      }),
    );
  }
  root.replaceChildren(title, errors, form, report, sources);
  void listSources().catch(showError);
  void listPets().catch(showError);
  const stop = await observeSnapshot(api.settingsEvents, api.settings, render, (e) => {
    showError(e);
    errors.append(
      button('重试读取设置', () => {
        void api.settings().then(render).catch(showError);
      }),
    );
  });
  return () => {
    stopped = true;
    stop();
  };
}
