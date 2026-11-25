import { createHash } from 'node:crypto';
import { readdir, readFile, stat } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import {
  SUPPORTED_TARGETS,
  binaryMatchesTarget,
  detectBinaryKind,
  sha256File,
} from './binary-arch.mjs';

const DEFAULT_REPO_ROOT = resolve(fileURLToPath(new URL('../..', import.meta.url)));

const IMMUTABLE_SOURCE_HASHES = {
  'yueying-cape-dancer/package/pet.json':
    '1e921e3bb634b97def071b6c06a8941f558c98c9757bb7726d38d9292d0dfa3f',
  'yueying-cape-dancer/package/spritesheet.webp':
    '70a06d18162ad3b5344afdc7e28d4dbd70b70c0d32ab0bfcac52fe7315b63130',
};

const REQUIRED_WINDOWS_INSTALL_CHECKS = [
  ['noConsoleFlash', 'Windows 转发器无控制台闪烁'],
  ['freshUserDir', '全新 Windows 用户目录安装'],
  ['webview2MissingOrFailed', 'Windows WebView2 缺失/下载失败处理'],
  ['chineseSpaceInstallPath', '中文及空格路径安装'],
  ['upgradeOldHookPath', '升级后旧 Hook 路径迁移'],
  ['uninstallPreservesThirdParty', '卸载保留第三方配置'],
];

const REQUIRED_FAULT_DRILLS = [
  ['diskWriteFailure', '磁盘写入失败演练'],
  ['lostAckRetry', '响应丢失重试去重演练'],
  ['restartRecovery', '重启恢复演练'],
  ['journalCompaction', '历史压缩保留去重演练'],
  ['sourceUpgradeMismatch', '来源升级失配演练'],
];

async function pathStat(p) {
  try {
    return await stat(p);
  } catch {
    return null;
  }
}

async function readJsonSafe(filePath) {
  try {
    const raw = await readFile(filePath, 'utf8');
    return { ok: true, value: JSON.parse(raw) };
  } catch (error) {
    return { ok: false, error: error instanceof Error ? error.message : String(error) };
  }
}

async function matchesHashAfterCrlfNormalization(filePath, expectedHash) {
  const raw = await readFile(filePath);
  if (!raw.includes(0x0d)) return false;
  const normalized = Buffer.from(raw.toString('utf8').replace(/\r\n/g, '\n'), 'utf8');
  return createHash('sha256').update(normalized).digest('hex') === expectedHash;
}

async function listEntriesRecursive(dir, maxDepth = 3, depth = 0, prefix = '') {
  const results = [];
  if (depth > maxDepth) return results;
  const entries = await readdir(dir, { withFileTypes: true }).catch(() => []);
  for (const entry of entries) {
    const rel = prefix ? `${prefix}/${entry.name}` : entry.name;
    results.push({ rel, name: entry.name, isDir: entry.isDirectory(), isFile: entry.isFile() });
    if (entry.isDirectory() && !entry.name.endsWith('.app')) {
      results.push(...(await listEntriesRecursive(join(dir, entry.name), maxDepth, depth + 1, rel)));
    }
  }
  return results;
}

async function validateTargetArtifacts(targetTriple, artifactsRoot, petOutputDir, errors) {
  const spec = SUPPORTED_TARGETS[targetTriple];
  const targetDir = join(artifactsRoot, targetTriple);
  const targetStat = await pathStat(targetDir);
  if (!targetStat || !targetStat.isDirectory()) {
    errors.push(`缺少目标架构产物目录：${targetTriple}`);
    return;
  }

  const entries = await listEntriesRecursive(targetDir, 3);
  if (spec.os === 'macos') {
    const hasApp = entries.some((e) => e.isDir && e.name.endsWith('.app'));
    const hasDmg = entries.some((e) => e.isFile && e.name.endsWith('.dmg'));
    if (!hasApp) errors.push(`${targetTriple} 缺少 macOS .app 应用包`);
    if (!hasDmg) errors.push(`${targetTriple} 缺少 macOS .dmg 镜像包`);
  } else if (spec.os === 'windows') {
    const hasNsis = entries.some(
      (e) => e.isFile && e.name.toLowerCase().endsWith('.exe') && e.name.toLowerCase() !== 'pet-bridge.exe',
    );
    if (!hasNsis) errors.push(`${targetTriple} 缺少 Windows x64 NSIS 安装包 (.exe)`);
  }

  // 检查独立或随包 pet-bridge
  const bridgeCandidates = [
    join(targetDir, spec.bridgeFileName),
    ...entries
      .filter((e) => e.isFile && e.name === spec.bridgeFileName)
      .map((e) => join(targetDir, e.rel)),
  ];
  let bridgePath = null;
  for (const candidate of bridgeCandidates) {
    const st = await pathStat(candidate);
    if (st && st.isFile()) {
      bridgePath = candidate;
      break;
    }
  }
  if (!bridgePath) {
    errors.push(`${targetTriple} 缺少独立/随包转发器 ${spec.bridgeFileName}`);
  } else {
    const kind = await detectBinaryKind(bridgePath);
    if (!binaryMatchesTarget(kind, targetTriple)) {
      errors.push(
        `${targetTriple} 的 ${spec.bridgeFileName} 架构不匹配：期望 ${spec.expectedBinaryKind}，实际 ${kind}`,
      );
    }
  }

  // 检查资源清单及关键随包资源
  const manifestPath = join(targetDir, 'resource-manifest.json');
  const manifestRead = await readJsonSafe(manifestPath);
  if (!manifestRead.ok) {
    errors.push(`${targetTriple} 缺少或无法解析资源清单 resource-manifest.json`);
    return;
  }
  const manifest = manifestRead.value;
  if (manifest.target !== targetTriple) {
    errors.push(`${targetTriple} 资源清单目标不匹配：manifest.target=${String(manifest.target)}`);
  }
  if (manifest.unsignedTestBuild !== true) {
    errors.push(`${targetTriple} 资源清单未明确标注测试产物未签名（unsignedTestBuild: true）`);
  }
  if (!manifest.bridge?.sha256 || manifest.bridge?.fileName !== spec.bridgeFileName) {
    errors.push(`${targetTriple} 资源清单缺少有效的转发器摘要记录`);
  } else if (bridgePath) {
    const actualBridgeSha = await sha256File(bridgePath);
    if (actualBridgeSha !== manifest.bridge.sha256) {
      errors.push(`${targetTriple} 的 ${spec.bridgeFileName} 哈希与资源清单不一致`);
    }
  }

  // 检查随包 default-pet 与 harness-bundle 文件
  const requiredFiles = [
    ['default-pet/pet.json', manifest.defaultPet?.files?.['pet.json']],
    ['default-pet/spritesheet.webp', manifest.defaultPet?.files?.['spritesheet.webp']],
    ['harness-bundle/dist/index.js', manifest.harnessBundle?.files?.['dist/index.js']],
    ['harness-bundle/cordis.patch.json', manifest.harnessBundle?.files?.['cordis.patch.json']],
    ['harness-bundle/package.json', manifest.harnessBundle?.files?.['package.json']],
  ];
  for (const [rel, expectedSha] of requiredFiles) {
    const full = join(targetDir, rel);
    const st = await pathStat(full);
    if (!st || !st.isFile()) {
      errors.push(`${targetTriple} 缺少随包资源 ${rel}`);
      continue;
    }
    if (!expectedSha) {
      errors.push(`${targetTriple} 资源清单缺少 ${rel} 的哈希记录`);
      continue;
    }
    const actual = await sha256File(full);
    if (actual !== expectedSha) {
      errors.push(`${targetTriple} 随包资源 ${rel} 哈希校验失败`);
    }
  }

  // 检查源 pet-output 未被覆盖
  const sourceChecksums = manifest.sourcePetOutputChecksums;
  if (!sourceChecksums || typeof sourceChecksums !== 'object') {
    errors.push(`${targetTriple} 资源清单缺少源 pet-output 校验和记录`);
  } else {
    for (const [rel, expectedImmutableHash] of Object.entries(IMMUTABLE_SOURCE_HASHES)) {
      if (sourceChecksums[rel] !== expectedImmutableHash) {
        errors.push(`${targetTriple} 记录的源素材 ${rel} 校验和与不可变基线不一致`);
      }
      const actualSrc = join(petOutputDir, rel);
      const st = await pathStat(actualSrc);
      if (!st || !st.isFile()) {
        errors.push(`源素材目录 pet-output 缺少 ${rel}`);
      } else {
        const diskHash = await sha256File(actualSrc);
        if (diskHash !== expectedImmutableHash || diskHash !== sourceChecksums[rel]) {
          const crlfHint =
            rel.endsWith('.json') &&
            (await matchesHashAfterCrlfNormalization(actualSrc, expectedImmutableHash))
              ? '（检测到换行符被转换为 CRLF，请按 .gitattributes 以 LF 重新检出）'
              : '';
          errors.push(`检测到源素材 pet-output/${rel} 被覆盖或修改（哈希不匹配）${crlfHint}`);
        }
      }
    }
  }
}

async function validateCompatibilityReports(reportsRoot, errors) {
  const codexPath = join(reportsRoot, 'codex-compatibility.json');
  const harnessPath = join(reportsRoot, 'harness-compatibility.json');

  const codex = await readJsonSafe(codexPath);
  if (!codex.ok) {
    errors.push('缺少 Codex 兼容版本报告（codex-compatibility.json）');
  } else {
    const hosts = codex.value?.hosts;
    const hasVersion =
      Array.isArray(hosts) &&
      hosts.some((h) => typeof h?.version === 'string' || typeof h?.embeddedCliVersion === 'string');
    if (!hasVersion) {
      errors.push('Codex 兼容报告缺少宿主版本记录');
    }
  }

  const harness = await readJsonSafe(harnessPath);
  if (!harness.ok) {
    errors.push('缺少 DeepSeek Harness 兼容版本报告（harness-compatibility.json）');
  } else if (typeof harness.value?.hostVersion !== 'string' || !harness.value.hostVersion.trim()) {
    errors.push('DeepSeek Harness 兼容报告缺少宿主版本（hostVersion）');
  }
}

async function validateGuiAcceptanceReport(reportsRoot, errors) {
  const guiPath = join(reportsRoot, 'gui-acceptance.json');
  const parsed = await readJsonSafe(guiPath);
  if (!parsed.ok) {
    errors.push('缺少真实双平台 GUI 与宿主验收报告（gui-acceptance.json），不能将构建成功视为发行通过');
    return;
  }
  const report = parsed.value;
  const platforms = report?.platforms ?? {};

  const checkPlatform = (key, label) => {
    const p = platforms[key];
    if (!p || p.status !== 'passed') {
      errors.push(`缺少或未通过 ${label} 真实 GUI 验收（${key}.status=${String(p?.status ?? 'missing')}）`);
      return;
    }
    if (typeof p.unfocusedP95Ms !== 'number' || p.unfocusedP95Ms > 50) {
      errors.push(`${label} 未聚焦 60 秒动画 P95 未达标（需 ≤50 ms，实际 ${String(p.unfocusedP95Ms)}）`);
    }
    if (typeof p.e2eLatencyP95Ms !== 'number' || p.e2eLatencyP95Ms >= 500) {
      errors.push(`${label} 正常通知端到端延迟 P95 未达标（需 <500 ms，实际 ${String(p.e2eLatencyP95Ms)}）`);
    }
    if (p.realSleepWake !== 'passed') {
      errors.push(`${label} 真实系统休眠/唤醒恢复未通过`);
    }
    const modes = p.resourceModes5Min ?? {};
    for (const mode of ['idle', 'animation', 'hidden']) {
      if (modes[mode] !== 'passed') {
        errors.push(`${label} 5 分钟资源占用验收（${mode}）未通过`);
      }
    }
  };

  checkPlatform('macosArm64', 'macOS Apple Silicon (arm64)');
  checkPlatform('macosX64', 'macOS Intel (x86_64)');
  checkPlatform('windowsX64', 'Windows 10/11 (x64)');

  const win = platforms.windowsX64 ?? {};
  for (const [field, label] of REQUIRED_WINDOWS_INSTALL_CHECKS) {
    if (win[field] !== 'passed') {
      errors.push(`Windows 真实安装/运行用例未通过：${label}（windowsX64.${field}）`);
    }
  }

  const hosts = report?.hosts ?? {};
  for (const [source, label] of [
    ['codex', 'Codex'],
    ['deepseek', 'DeepSeek Harness'],
  ]) {
    const h = hosts[source] ?? {};
    if (typeof h.macosRealSessions !== 'number' || h.macosRealSessions < 3) {
      errors.push(`${label} macOS 真实会话精确导航未达标（需 ≥3，实际 ${String(h.macosRealSessions ?? 0)}）`);
    }
    if (typeof h.windowsRealSessions !== 'number' || h.windowsRealSessions < 3) {
      errors.push(`${label} Windows 真实会话精确导航未达标（需 ≥3，实际 ${String(h.windowsRealSessions ?? 0)}）`);
    }
  }

  const drills = report?.faultDrills ?? {};
  for (const [field, label] of REQUIRED_FAULT_DRILLS) {
    if (drills[field] !== 'passed') {
      errors.push(`故障演练未通过：${label}（faultDrills.${field}）`);
    }
  }
}

export async function validateRelease(options) {
  if (!options?.artifactsDir || !options?.reportsDir) {
    throw new Error('必须提供 --artifacts 与 --reports 目录参数');
  }
  const repoRoot = resolve(options.repoRoot ?? DEFAULT_REPO_ROOT);
  const artifactsDir = resolve(options.artifactsDir);
  const reportsDir = resolve(options.reportsDir);
  const petOutputDir = resolve(options.petOutputDir ?? join(repoRoot, 'pet-output'));

  const errors = [];
  for (const targetTriple of Object.keys(SUPPORTED_TARGETS)) {
    await validateTargetArtifacts(targetTriple, artifactsDir, petOutputDir, errors);
  }
  await validateCompatibilityReports(reportsDir, errors);
  await validateGuiAcceptanceReport(reportsDir, errors);

  return {
    ok: errors.length === 0,
    errors,
    checkedTargets: Object.keys(SUPPORTED_TARGETS),
  };
}

function parseCliArgs(argv) {
  const options = {};
  const map = {
    '--artifacts': 'artifactsDir',
    '--reports': 'reportsDir',
    '--pet-output': 'petOutputDir',
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
    const result = await validateRelease(options);
    if (result.ok) {
      console.log(`发行验收校验通过（覆盖目标：${result.checkedTargets.join(', ')}）`);
    } else {
      console.error(`发行验收校验未通过，共 ${result.errors.length} 项缺口：`);
      for (const item of result.errors) {
        console.error(`- ${item}`);
      }
      process.exitCode = 1;
    }
  } catch (error) {
    console.error(`发行校验参数或运行错误：${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
  }
}
