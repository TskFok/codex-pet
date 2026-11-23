// 发行构建时生成固定资源。桌宠在用户明确操作时仅替换安装 ID 和桥接器绝对路径。
import { mkdtemp, mkdir, rename, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createInstallPackage } from './create-install.mjs';

const root = fileURLToPath(new URL('./prepared/', import.meta.url));
await mkdir(root, { recursive: true });
const staging = await mkdtemp(join(root, '.stage-'));
try {
  await createInstallPackage({
    outputDir: join(staging, 'harness-bundle'),
    bridgePath: join(root, 'pet-bridge'),
    installationId: '00000000-0000-4000-8000-000000000000',
  });
  await rm(join(root, 'harness-bundle'), { recursive: true, force: true });
  await rename(join(staging, 'harness-bundle'), join(root, 'harness-bundle'));
} finally {
  await rm(staging, { recursive: true, force: true });
}
