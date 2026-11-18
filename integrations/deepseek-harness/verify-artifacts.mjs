import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
const host = await import('./dist/index.js');
if (typeof host.apply !== 'function') throw new Error('Host插件没有可加载的apply入口');
let loaded;
vm.runInNewContext(await readFile(new URL('./dist/client.js', import.meta.url), 'utf8'), {
  window: { __ModuleLoader__: { load(row) { loaded = row; } } },
});
if (loaded?.id !== '@codex-pet/deepseek-integration') throw new Error('Client没有注册到宿主模块加载器');
const client = loaded.factory(() => { throw new Error('Client包含未声明的运行时依赖'); });
if (typeof client.apply !== 'function') throw new Error('Client插件缺少apply入口');
console.log('Host与Client构建入口可加载');
