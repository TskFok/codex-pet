import { build } from 'esbuild';
// 宿主公开模块加载器格式；没有私有运行时或未声明的模块表依赖。
await build({ entryPoints: ['src/client.ts'], bundle: true, platform: 'browser', format: 'cjs', outfile: 'dist/client.js',
  banner: { js: 'window.__ModuleLoader__.load({ id: "@codex-pet/deepseek-integration", factory: (require) => { const module = { exports: {} }; const exports = module.exports;' },
  footer: { js: 'return module.exports; }});' }, target: 'es2023' });
