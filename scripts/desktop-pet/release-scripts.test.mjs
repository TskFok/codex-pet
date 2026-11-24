import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdir, mkdtemp, readFile, rm, writeFile, cp, chmod } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(fileURLToPath(new URL('../..', import.meta.url)));
const validateScript = join(repoRoot, 'scripts/desktop-pet/validate-release.mjs');
const prepareScript = join(repoRoot, 'scripts/desktop-pet/prepare-resources.mjs');

function makeMachOHeader(arch) {
  const buf = Buffer.alloc(64, 0);
  // MH_MAGIC_64 (little-endian: 0xfeedfacf)
  buf.writeUInt32LE(0xfeedfacf, 0);
  // CPU_TYPE_ARM64 = 0x0100000c, CPU_TYPE_X86_64 = 0x01000007
  buf.writeUInt32LE(arch === 'arm64' ? 0x0100000c : 0x01000007, 4);
  return buf;
}

function makePeX64Header() {
  const buf = Buffer.alloc(128, 0);
  buf.write('MZ', 0, 'ascii');
  buf.writeUInt32LE(0x40, 0x3c); // e_lfanew = 0x40
  buf.write('PE\0\0', 0x40, 'ascii');
  buf.writeUInt16LE(0x8664, 0x44); // IMAGE_FILE_MACHINE_AMD64
  return buf;
}

function sha256(buf) {
  return createHash('sha256').update(buf).digest('hex');
}

async function createFakeCompleteRelease(baseDir, opts = {}) {
  const artifacts = join(baseDir, 'artifacts');
  const reports = join(baseDir, 'reports');
  const petOutput = join(baseDir, 'pet-output');
  await mkdir(artifacts, { recursive: true });
  await mkdir(reports, { recursive: true });
  await mkdir(join(petOutput, 'yueying-cape-dancer/package'), { recursive: true });

  const petJsonBytes = await readFile(join(repoRoot, 'pet-output/yueying-cape-dancer/package/pet.json'));
  const webpBytes = await readFile(join(repoRoot, 'pet-output/yueying-cape-dancer/package/spritesheet.webp'));
  await writeFile(join(petOutput, 'yueying-cape-dancer/package/pet.json'), petJsonBytes);
  await writeFile(join(petOutput, 'yueying-cape-dancer/package/spritesheet.webp'), webpBytes);

  const targets = [
    {
      triple: 'aarch64-apple-darwin',
      bridgeName: 'pet-bridge',
      bridgeBytes: makeMachOHeader(opts.wrongArm64Arch ? 'x86_64' : 'arm64'),
      bundleFiles: ['Codex Pet.app/Contents/Info.plist', 'Codex Pet_0.1.0_aarch64.dmg'],
    },
    {
      triple: 'x86_64-apple-darwin',
      bridgeName: 'pet-bridge',
      bridgeBytes: makeMachOHeader('x86_64'),
      bundleFiles: ['Codex Pet.app/Contents/Info.plist', 'Codex Pet_0.1.0_x64.dmg'],
    },
    {
      triple: 'x86_64-pc-windows-msvc',
      bridgeName: 'pet-bridge.exe',
      bridgeBytes: makePeX64Header(),
      bundleFiles: ['Codex Pet_0.1.0_x64-setup.exe'],
    },
  ];

  for (const t of targets) {
    const dir = join(artifacts, t.triple);
    await mkdir(join(dir, 'harness-bundle/dist'), { recursive: true });
    await mkdir(join(dir, 'default-pet'), { recursive: true });
    for (const rel of t.bundleFiles) {
      const full = join(dir, rel);
      await mkdir(resolve(full, '..'), { recursive: true });
      await writeFile(full, 'placeholder-artifact');
    }
    const indexJs = Buffer.from('export const ok = true;\n');
    const patchJson = Buffer.from('[{"insert":[{"id":"desktop-pet-notify","name":"@codex-pet/deepseek-integration","config":{"installationId":"00000000-0000-4000-8000-000000000000","bridgePath":"/abs/pet-bridge"}}]}]\n');
    const pkgJson = Buffer.from('{"name":"@codex-pet/deepseek-integration","dsh":{"bundle":{"patch":"./cordis.patch.json"}}}\n');
    await writeFile(join(dir, 'harness-bundle/dist/index.js'), indexJs);
    await writeFile(join(dir, 'harness-bundle/cordis.patch.json'), patchJson);
    await writeFile(join(dir, 'harness-bundle/package.json'), pkgJson);
    await writeFile(join(dir, 'default-pet/pet.json'), petJsonBytes);
    await writeFile(join(dir, 'default-pet/spritesheet.webp'), webpBytes);

    if (!opts.omitWindowsBridge || t.triple !== 'x86_64-pc-windows-msvc') {
      await writeFile(join(dir, t.bridgeName), t.bridgeBytes);
    }

    const manifest = {
      schemaVersion: 1,
      target: t.triple,
      unsignedTestBuild: true,
      bridge: {
        fileName: t.bridgeName,
        target: t.triple,
        sha256: sha256(t.bridgeBytes),
      },
      defaultPet: {
        id: 'yueying-cape-dancer',
        files: {
          'pet.json': sha256(petJsonBytes),
          'spritesheet.webp': sha256(webpBytes),
        },
      },
      harnessBundle: {
        files: {
          'dist/index.js': sha256(indexJs),
          'cordis.patch.json': sha256(patchJson),
          'package.json': sha256(pkgJson),
        },
      },
      sourcePetOutputChecksums: {
        'yueying-cape-dancer/package/pet.json': sha256(petJsonBytes),
        'yueying-cape-dancer/package/spritesheet.webp': sha256(webpBytes),
      },
    };
    await writeFile(join(dir, 'resource-manifest.json'), JSON.stringify(manifest, null, 2));
  }

  if (opts.mutateSourcePetOutput) {
    await writeFile(join(petOutput, 'yueying-cape-dancer/package/pet.json'), '{"mutated":true}');
  }

  if (!opts.omitCompatReports) {
    await cp(join(repoRoot, 'integrations/codex/compatibility.json'), join(reports, 'codex-compatibility.json'));
    await cp(join(repoRoot, 'integrations/deepseek-harness/compatibility.json'), join(reports, 'harness-compatibility.json'));
  }

  const guiReport = {
    schemaVersion: 1,
    platforms: {
      macosArm64: {
        status: 'passed',
        unfocusedP95Ms: 35,
        e2eLatencyP95Ms: 120,
        realSleepWake: 'passed',
        resourceModes5Min: { idle: 'passed', animation: 'passed', hidden: 'passed' },
      },
      macosX64: {
        status: 'passed',
        unfocusedP95Ms: 38,
        e2eLatencyP95Ms: 140,
        realSleepWake: 'passed',
        resourceModes5Min: { idle: 'passed', animation: 'passed', hidden: 'passed' },
      },
      windowsX64: {
        status: opts.windowsGuiPending ? 'unverified' : 'passed',
        unfocusedP95Ms: 40,
        e2eLatencyP95Ms: 180,
        realSleepWake: 'passed',
        noConsoleFlash: 'passed',
        freshUserDir: 'passed',
        webview2MissingOrFailed: 'passed',
        chineseSpaceInstallPath: 'passed',
        upgradeOldHookPath: 'passed',
        uninstallPreservesThirdParty: 'passed',
        resourceModes5Min: { idle: 'passed', animation: 'passed', hidden: 'passed' },
      },
    },
    hosts: {
      codex: { macosRealSessions: 3, windowsRealSessions: 3 },
      deepseek: { macosRealSessions: 3, windowsRealSessions: 3 },
    },
    faultDrills: {
      diskWriteFailure: 'passed',
      lostAckRetry: 'passed',
      restartRecovery: 'passed',
      journalCompaction: 'passed',
      sourceUpgradeMismatch: 'passed',
    },
  };
  if (!opts.omitGuiReport) {
    await writeFile(join(reports, 'gui-acceptance.json'), JSON.stringify(guiReport, null, 2));
  }
  return { artifacts, reports, petOutput };
}

test('validate-release 拒绝仓库中的不完整夹具并明确列出缺项', () => {
  const res = spawnSync(
    process.execPath,
    [
      validateScript,
      '--artifacts',
      join(repoRoot, 'tests/fixtures/release/incomplete'),
      '--reports',
      join(repoRoot, 'tests/fixtures/release/incomplete-reports'),
    ],
    { encoding: 'utf8' },
  );
  assert.notEqual(res.status, 0, '不完整发行包必须退出非零');
  const output = `${res.stdout}\n${res.stderr}`;
  assert.match(output, /x86_64-apple-darwin/);
  assert.match(output, /x86_64-pc-windows-msvc/);
  assert.match(output, /pet-bridge/);
  assert.match(output, /兼容/);
  assert.match(output, /GUI|Windows|真实会话/);
});

test('validate-release 分别拒绝缺转发器、错架构、缺 GUI 报告、缺兼容版本和源素材被篡改', async () => {
  const base = await mkdtemp(join(tmpdir(), 'pet-release-val-'));
  try {
    // 完整且全部真实验收通过时应退出 0
    const completeDir = join(base, 'complete');
    const complete = await createFakeCompleteRelease(completeDir);
    const ok = spawnSync(
      process.execPath,
      [
        validateScript,
        '--artifacts', complete.artifacts,
        '--reports', complete.reports,
        '--pet-output', complete.petOutput,
      ],
      { encoding: 'utf8' },
    );
    assert.equal(ok.status, 0, `完整用例应通过，实际输出: ${ok.stdout}\n${ok.stderr}`);

    const failureCases = [
      ['缺转发器', { omitWindowsBridge: true }, /pet-bridge\.exe/],
      ['错架构', { wrongArm64Arch: true }, /架构|arm64/],
      ['缺 Windows 真实 GUI 报告', { windowsGuiPending: true }, /windowsX64|Windows/],
      ['缺兼容版本报告', { omitCompatReports: true }, /兼容/],
      ['源码素材被覆盖', { mutateSourcePetOutput: true }, /pet-output|校验|哈希/],
    ];
    for (const [label, opts, pattern] of failureCases) {
      const caseDir = join(base, label.replace(/\s+/g, '-'));
      const fixture = await createFakeCompleteRelease(caseDir, opts);
      const res = spawnSync(
        process.execPath,
        [
          validateScript,
          '--artifacts', fixture.artifacts,
          '--reports', fixture.reports,
          '--pet-output', fixture.petOutput,
        ],
        { encoding: 'utf8' },
      );
      assert.notEqual(res.status, 0, `${label} 必须失败`);
      assert.match(`${res.stdout}\n${res.stderr}`, pattern, `${label} 应指出具体缺项`);
    }
  } finally {
    await rm(base, { recursive: true, force: true });
  }
});

test('prepare-resources 只复制发行所需素材到构建目录，校验转发器架构且不修改原 pet-output', async () => {
  const base = await mkdtemp(join(tmpdir(), 'pet-prepare-'));
  try {
    const outDir = join(base, 'stage');
    const fakeBridge = join(base, 'pet-bridge-bin');
    await writeFile(fakeBridge, makeMachOHeader('arm64'));
    await chmod(fakeBridge, 0o755);

    const fakeTemplate = join(base, 'template');
    await mkdir(join(fakeTemplate, 'dist'), { recursive: true });
    await writeFile(join(fakeTemplate, 'dist/index.js'), 'export default {};\n');
    await writeFile(join(fakeTemplate, 'package.json'), '{"name":"@codex-pet/deepseek-integration","dsh":{"bundle":{"patch":"./cordis.patch.json"}}}\n');
    await writeFile(join(fakeTemplate, 'cordis.patch.json'), '[{"insert":[{"id":"desktop-pet-notify","name":"@codex-pet/deepseek-integration","config":{"installationId":"00000000-0000-4000-8000-000000000000","bridgePath":"/tmp/pet-bridge"}}]}]\n');
    await writeFile(join(fakeTemplate, 'README.md'), '# template\n');
    await writeFile(join(fakeTemplate, 'compatibility.json'), '{"manifestVersion":1}\n');

    const beforePetJson = sha256(await readFile(join(repoRoot, 'pet-output/yueying-cape-dancer/package/pet.json')));
    const beforeWebp = sha256(await readFile(join(repoRoot, 'pet-output/yueying-cape-dancer/package/spritesheet.webp')));

    execFileSync(
      process.execPath,
      [
        prepareScript,
        '--target', 'aarch64-apple-darwin',
        '--bridge', fakeBridge,
        '--harness-template', fakeTemplate,
        '--output', outDir,
      ],
      { encoding: 'utf8' },
    );

    const afterPetJson = sha256(await readFile(join(repoRoot, 'pet-output/yueying-cape-dancer/package/pet.json')));
    const afterWebp = sha256(await readFile(join(repoRoot, 'pet-output/yueying-cape-dancer/package/spritesheet.webp')));
    assert.equal(afterPetJson, beforePetJson);
    assert.equal(afterWebp, beforeWebp);

    const manifest = JSON.parse(await readFile(join(outDir, 'resources/resource-manifest.json'), 'utf8'));
    assert.equal(manifest.target, 'aarch64-apple-darwin');
    assert.equal(manifest.defaultPet.files['spritesheet.webp'], beforeWebp);
    assert.equal(manifest.bridge.fileName, 'pet-bridge');

    // 传入错架构转发器必须拒绝
    const wrongBridge = join(base, 'wrong-arch-bridge');
    await writeFile(wrongBridge, makeMachOHeader('x86_64'));
    const bad = spawnSync(
      process.execPath,
      [
        prepareScript,
        '--target', 'aarch64-apple-darwin',
        '--bridge', wrongBridge,
        '--harness-template', fakeTemplate,
        '--output', join(base, 'stage-bad'),
      ],
      { encoding: 'utf8' },
    );
    assert.notEqual(bad.status, 0, '错架构桥接器必须被 prepare-resources 拒绝');
  } finally {
    await rm(base, { recursive: true, force: true });
  }
});
