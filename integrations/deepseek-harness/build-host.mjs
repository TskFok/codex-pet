import { rm } from 'node:fs/promises';
import { build } from 'esbuild';

// Host 入口打包为自包含 ESM：内联 zod，避免宿主以 link: 方式加载离线目录时找不到 node_modules 依赖；
// peerDependencies 中的 @deepseek-ai/* 始终保持 external，避免重复打包宿主共享实例。
await rm(new URL('./dist', import.meta.url), { recursive: true, force: true });

const shared = {
  bundle: true,
  platform: 'node',
  format: 'esm',
  target: 'node24',
  external: ['@deepseek-ai/*'],
};

await build({ ...shared, entryPoints: ['src/index.ts'], outfile: 'dist/index.js' });
await build({ ...shared, entryPoints: ['src/open-session.ts'], outfile: 'dist/open-session.js' });
