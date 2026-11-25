import { cp, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import vm from 'node:vm';

const distDir = fileURLToPath(new URL('./dist', import.meta.url));
const isolated = await mkdtemp(join(tmpdir(), 'dsh-verify-'));
try {
  await writeFile(join(isolated, 'package.json'), '{"type":"module"}\n');
  await cp(join(distDir, 'index.js'), join(isolated, 'index.js'));
  await cp(join(distDir, 'open-session.js'), join(isolated, 'open-session.js'));

  const host = await import(pathToFileURL(join(isolated, 'index.js')).href);
  if (typeof host.apply !== 'function') throw new Error('Host插件没有可加载的apply入口');
  if (typeof host.Config?.parse !== 'function') throw new Error('Host插件缺少Config校验schema');
  host.Config.parse({
    bridgePath: resolve('pet-bridge'),
    installationId: '11111111-1111-4111-8111-111111111111',
  });

  const openSession = await import(pathToFileURL(join(isolated, 'open-session.js')).href);
  if (typeof openSession.openHarnessSession !== 'function') {
    throw new Error('open-session构建产物缺少openHarnessSession入口');
  }
} finally {
  await rm(isolated, { recursive: true, force: true });
}

let loaded;
vm.runInNewContext(await readFile(new URL('./dist/client.js', import.meta.url), 'utf8'), {
  window: { __ModuleLoader__: { load(row) { loaded = row; } } },
});
if (loaded?.id !== '@codex-pet/deepseek-integration') throw new Error('Client没有注册到宿主模块加载器');
const client = loaded.factory(() => { throw new Error('Client包含未声明的运行时依赖'); });
if (typeof client.apply !== 'function') throw new Error('Client插件缺少apply入口');
console.log('Host与Client构建入口可在脱离node_modules的隔离目录独立加载');
