import { execFileSync } from 'node:child_process';
import { chmod, copyFile, mkdir, mkdtemp, readdir, rename, rm, stat, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import {
  SUPPORTED_TARGETS,
  binaryMatchesTarget,
  detectBinaryKind,
  hostDefaultTarget,
  sha256File,
} from './binary-arch.mjs';

const DEFAULT_REPO_ROOT = resolve(fileURLToPath(new URL('../..', import.meta.url)));

const DEFAULT_PET_ID = 'yueying-cape-dancer';
const DEFAULT_PET_FILES = ['pet.json', 'spritesheet.webp'];
const HARNESS_TEMPLATE_FILES = ['package.json', 'cordis.patch.json', 'README.md', 'compatibility.json'];

async function pathExists(p) {
  try {
    await stat(p);
    return true;
  } catch {
    return false;
  }
}

async function hashDirectoryFiles(rootDir, prefix = '') {
  const entries = await readdir(rootDir, { withFileTypes: true });
  const hashes = {};
  for (const entry of entries.sort((a, b) => a.name.localeCompare(b.name))) {
    const fullPath = join(rootDir, entry.name);
    const relPath = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.isDirectory()) {
      Object.assign(hashes, await hashDirectoryFiles(fullPath, relPath));
    } else if (entry.isFile()) {
      hashes[relPath] = await sha256File(fullPath);
    }
  }
  return hashes;
}

async function resolveOrBuildBridge({ repoRoot, target, explicitBridge, profile }) {
  const spec = SUPPORTED_TARGETS[target];
  if (explicitBridge) {
    const abs = resolve(explicitBridge);
    if (!(await pathExists(abs))) {
      throw new Error(`指定的 pet-bridge 不存在：${abs}`);
    }
    return abs;
  }

  const cargoProfile = profile === 'debug' ? 'debug' : 'release';
  const targetCandidate = join(repoRoot, 'target', target, cargoProfile, spec.bridgeFileName);
  const hostCandidate = join(repoRoot, 'target', cargoProfile, spec.bridgeFileName);

  const cargoArgs = ['build', '-p', 'pet-bridge', '--locked', '--target', target];
  if (cargoProfile === 'release') cargoArgs.push('--release');
  execFileSync('cargo', cargoArgs, { cwd: repoRoot, stdio: 'inherit' });

  if (await pathExists(targetCandidate)) return targetCandidate;
  if (await pathExists(hostCandidate)) return hostCandidate;
  throw new Error(`构建 pet-bridge 后未找到目标产物：${targetCandidate}`);
}

function resolvePnpmInvocation(args) {
  if (process.platform !== 'win32') return { command: 'pnpm', args };
  const npmExecPath = process.env.npm_execpath;
  if (npmExecPath) {
    const lower = npmExecPath.toLowerCase();
    if (lower.endsWith('.js') || lower.endsWith('.cjs') || lower.endsWith('.mjs')) {
      return { command: process.execPath, args: [npmExecPath, ...args] };
    }
    if (lower.endsWith('.exe') || lower.endsWith('.com')) {
      return { command: npmExecPath, args };
    }
  }
  return { command: process.env.ComSpec || 'cmd.exe', args: ['/d', '/s', '/c', 'pnpm', ...args] };
}

async function resolveOrBuildHarnessTemplate({ repoRoot, explicitTemplate }) {
  if (explicitTemplate) {
    const abs = resolve(explicitTemplate);
    if (!(await pathExists(join(abs, 'dist/index.js')))) {
      throw new Error(`指定的 Harness 配置包模板缺少 dist/index.js：${abs}`);
    }
    return abs;
  }

  const integrationDir = join(repoRoot, 'integrations/deepseek-harness');
  const distIndex = join(integrationDir, 'dist/index.js');
  if (!(await pathExists(distIndex))) {
    const invocation = resolvePnpmInvocation(['--filter', '@codex-pet/deepseek-integration', 'build']);
    execFileSync(invocation.command, invocation.args, {
      cwd: repoRoot,
      stdio: 'inherit',
    });
  }
  execFileSync(process.execPath, [join(integrationDir, 'prepare-template.mjs')], {
    cwd: repoRoot,
    stdio: 'inherit',
  });
  const prepared = join(integrationDir, 'prepared/harness-bundle');
  if (!(await pathExists(join(prepared, 'dist/index.js')))) {
    throw new Error(`未能生成 Harness 离线模板：${prepared}`);
  }
  return prepared;
}

/**
 * 仅复制发行所需素材与目标架构 pet-bridge 到构建目录，绝不修改 pet-output/ 原包。
 */
export async function prepareResources(options = {}) {
  const repoRoot = resolve(options.repoRoot ?? DEFAULT_REPO_ROOT);
  const target = options.target ?? process.env.TAURI_ENV_TARGET_TRIPLE ?? hostDefaultTarget();
  const spec = SUPPORTED_TARGETS[target];
  if (!spec) {
    throw new Error(`不支持的发行目标架构：${target}（支持：${Object.keys(SUPPORTED_TARGETS).join(', ')}）`);
  }

  const profile =
    options.profile ?? (process.env.TAURI_ENV_DEBUG === 'true' ? 'debug' : 'release');
  const outputDir = resolve(options.outputDir ?? join(repoRoot, 'apps/desktop/src-tauri'));
  const petOutputDir = resolve(options.petOutputDir ?? join(repoRoot, 'pet-output'));
  const petPackageDir = join(petOutputDir, DEFAULT_PET_ID, 'package');

  // 复制前记录源 pet-output 校验和，结束后再次核对未发生任何写入。
  const sourceBefore = {};
  for (const file of DEFAULT_PET_FILES) {
    const srcPath = join(petPackageDir, file);
    if (!(await pathExists(srcPath))) {
      throw new Error(`缺少默认宠物源素材：${srcPath}`);
    }
    sourceBefore[`${DEFAULT_PET_ID}/package/${file}`] = await sha256File(srcPath);
  }

  const bridgeSource = await resolveOrBuildBridge({
    repoRoot,
    target,
    explicitBridge: options.bridgePath,
    profile,
  });
  const detectedKind = await detectBinaryKind(bridgeSource);
  if (!binaryMatchesTarget(detectedKind, target)) {
    throw new Error(
      `pet-bridge 架构不匹配：目标 ${target} 需要 ${spec.expectedBinaryKind}，实际检测到 ${detectedKind}（${bridgeSource}）`,
    );
  }

  const templateSource = await resolveOrBuildHarnessTemplate({
    repoRoot,
    explicitTemplate: options.harnessTemplateDir,
  });

  await mkdir(outputDir, { recursive: true });
  const stageRoot = await mkdtemp(join(outputDir, '.prepare-stage-'));
  try {
    const stageResources = join(stageRoot, 'resources');
    const stageDefaultPet = join(stageResources, 'default-pet');
    const stageHarness = join(stageResources, 'harness-bundle');
    const stageHarnessDist = join(stageHarness, 'dist');
    const stageBinaries = join(stageRoot, 'binaries');
    await mkdir(stageDefaultPet, { recursive: true });
    await mkdir(stageHarnessDist, { recursive: true });
    await mkdir(stageBinaries, { recursive: true });

    const defaultPetHashes = {};
    for (const file of DEFAULT_PET_FILES) {
      const src = join(petPackageDir, file);
      const dst = join(stageDefaultPet, file);
      await copyFile(src, dst);
      defaultPetHashes[file] = await sha256File(dst);
    }

    for (const file of HARNESS_TEMPLATE_FILES) {
      const src = join(templateSource, file);
      if (!(await pathExists(src))) {
        throw new Error(`Harness 模板缺少文件：${src}`);
      }
      await copyFile(src, join(stageHarness, file));
    }
    const distEntries = await readdir(join(templateSource, 'dist'), { withFileTypes: true });
    for (const entry of distEntries) {
      if (!entry.isFile()) {
        throw new Error(`Harness 模板 dist/ 包含非文件条目：${entry.name}`);
      }
      await copyFile(join(templateSource, 'dist', entry.name), join(stageHarnessDist, entry.name));
    }
    const harnessBundleHashes = await hashDirectoryFiles(stageHarness);

    const ext = spec.os === 'windows' ? '.exe' : '';
    const sidecarName = `pet-bridge-${target}${ext}`;
    const stageBridge = join(stageBinaries, sidecarName);
    await copyFile(bridgeSource, stageBridge);
    if (spec.os !== 'windows') {
      await chmod(stageBridge, 0o755);
    }
    const bridgeSha256 = await sha256File(stageBridge);

    // 校验源 pet-output 未被覆盖或修改
    for (const file of DEFAULT_PET_FILES) {
      const afterHash = await sha256File(join(petPackageDir, file));
      const key = `${DEFAULT_PET_ID}/package/${file}`;
      if (afterHash !== sourceBefore[key]) {
        throw new Error(`检测到源素材被修改：${key}`);
      }
    }

    const manifest = {
      schemaVersion: 1,
      target,
      profile,
      unsignedTestBuild: true,
      bridge: {
        fileName: spec.bridgeFileName,
        sidecarFileName: sidecarName,
        target,
        arch: spec.arch,
        binaryKind: detectedKind,
        sha256: bridgeSha256,
      },
      defaultPet: {
        id: DEFAULT_PET_ID,
        files: defaultPetHashes,
      },
      harnessBundle: {
        files: harnessBundleHashes,
      },
      sourcePetOutputChecksums: sourceBefore,
    };
    await writeFile(
      join(stageResources, 'resource-manifest.json'),
      `${JSON.stringify(manifest, null, 2)}\n`,
      'utf8',
    );

    const finalResources = join(outputDir, 'resources');
    const finalBinaries = join(outputDir, 'binaries');
    await mkdir(finalBinaries, { recursive: true });
    await rm(finalResources, { recursive: true, force: true });
    await rename(stageResources, finalResources);
    const finalSidecar = join(finalBinaries, sidecarName);
    await rm(finalSidecar, { force: true });
    await rename(stageBridge, finalSidecar);

    return {
      target,
      resourcesDir: finalResources,
      sidecarPath: finalSidecar,
      manifestPath: join(finalResources, 'resource-manifest.json'),
      manifest,
    };
  } finally {
    await rm(stageRoot, { recursive: true, force: true });
  }
}

function parseCliArgs(argv) {
  const options = {};
  const map = {
    '--target': 'target',
    '--bridge': 'bridgePath',
    '--harness-template': 'harnessTemplateDir',
    '--output': 'outputDir',
    '--pet-output': 'petOutputDir',
    '--profile': 'profile',
    '--repo-root': 'repoRoot',
  };
  for (let i = 0; i < argv.length; i += 2) {
    const key = map[argv[i]];
    if (!key || argv[i + 1] === undefined) {
      throw new Error(`无效参数：${argv[i] ?? ''}`);
    }
    options[key] = argv[i + 1];
  }
  return options;
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  try {
    const options = parseCliArgs(process.argv.slice(2));
    const result = await prepareResources(options);
    console.log(
      `已准备发行资源（目标 ${result.target}）：sidecar=${result.sidecarPath}，manifest=${result.manifestPath}`,
    );
  } catch (error) {
    console.error(`准备发行资源失败：${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
  }
}
