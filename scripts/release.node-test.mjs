import assert from "node:assert/strict";
import path from "node:path";
import { describe, it } from "node:test";
import {
  getConsistentVersion,
  parseReleaseArgs,
  resolveTargetVersion,
  updateVersionContents,
  VERSION_PATHS,
} from "./release-core.mjs";
import { resolvePnpmCommand, runRelease } from "./release.mjs";

describe("发布参数", () => {
  it("无参数时递增补丁号", () => {
    assert.equal(resolveTargetVersion(parseReleaseArgs([]), "0.1.9"), "0.1.10");
  });

  it("接受更高的显式稳定版本", () => {
    assert.equal(
      resolveTargetVersion(parseReleaseArgs(["1.2.3"]), "0.1.9"),
      "1.2.3",
    );
  });

  it("兼容 pnpm 透传的单个前导 -- 分隔符", () => {
    assert.deepEqual(parseReleaseArgs(["--"]), { mode: "next-patch" });
    assert.deepEqual(parseReleaseArgs(["--", "1.2.3"]), {
      mode: "explicit",
      version: "1.2.3",
    });
    assert.deepEqual(parseReleaseArgs(["--", "--current"]), {
      mode: "current",
    });
  });

  it("精确比较超出安全整数范围的版本段", () => {
    assert.equal(
      resolveTargetVersion(
        parseReleaseArgs(["1.0.9007199254740993"]),
        "1.0.9007199254740992",
      ),
      "1.0.9007199254740993",
    );
  });

  it("精确递增超出安全整数范围的补丁号", () => {
    assert.equal(
      resolveTargetVersion(parseReleaseArgs([]), "1.0.9007199254740992"),
      "1.0.9007199254740993",
    );
  });

  it("current 模式沿用当前版本", () => {
    assert.equal(
      resolveTargetVersion(parseReleaseArgs(["--current"]), "0.1.9"),
      "0.1.9",
    );
  });

  for (const version of ["v1.2.3", "1.2", "1.2.3-beta.1", "01.2.3"]) {
    it(`拒绝非法版本 ${version}`, () => {
      assert.throws(() => parseReleaseArgs([version]), /稳定 SemVer/);
    });
  }

  it("拒绝 current 与其他参数组合", () => {
    assert.throws(
      () => parseReleaseArgs(["--current", "1.2.3"]),
      /不能与其他参数组合/,
    );
  });

  it("拒绝多余位置参数或重复 --", () => {
    assert.throws(
      () => parseReleaseArgs(["1.2.3", "1.2.4"]),
      /pnpm release/,
    );
    assert.throws(
      () => parseReleaseArgs(["--", "--", "--current"]),
      /不能与其他参数组合/,
    );
  });

  for (const version of ["0.1.9", "0.1.8"]) {
    it(`拒绝相同或更低版本 ${version}`, () => {
      assert.throws(
        () => resolveTargetVersion(parseReleaseArgs([version]), "0.1.9"),
        /必须高于/,
      );
    });
  }
});

const manifests = {
  packageJson: '{\n  "name": "codex-pet",\n  "version": "0.1.0",\n  "private": true\n}\n',
  desktopPackageJson:
    '{\n  "name": "@codex-pet/desktop",\n  "version": "0.1.0",\n  "private": true\n}\n',
  harnessPackageJson:
    '{\n  "name": "@codex-pet/deepseek-integration",\n  "version": "0.1.0",\n  "private": true\n}\n',
  tauriConfig: '{\n  "productName": "Codex Pet",\n  "version": "0.1.0"\n}\n',
  desktopCargoToml:
    '[package]\nname = "pet-desktop"\nversion = "0.1.0"\nedition.workspace = true\n',
  coreCargoToml:
    '[package]\nname = "pet-core"\nversion = "0.1.0"\nedition.workspace = true\n',
  bridgeCargoToml:
    '[package]\nname = "pet-bridge"\nversion = "0.1.0"\nedition.workspace = true\n',
  cargoLock: `[[package]]
name = "pet-bridge"
version = "0.1.0"
dependencies = []

[[package]]
name = "pet-core"
version = "0.1.0"
dependencies = []

[[package]]
name = "pet-desktop"
version = "0.1.0"
dependencies = []
`,
};

describe("版本清单", () => {
  it("读取全部一致的版本源", () => {
    assert.equal(getConsistentVersion(manifests), "0.1.0");
  });

  it("报告不一致的文件和值", () => {
    const inconsistent = {
      ...manifests,
      tauriConfig: manifests.tauriConfig.replace("0.1.0", "0.2.0"),
    };
    assert.throws(
      () => getConsistentVersion(inconsistent),
      /apps\/desktop\/src-tauri\/tauri\.conf\.json=0\.2\.0/,
    );
  });

  it("报告 Cargo.lock 中单个工作区包版本不一致", () => {
    const inconsistent = {
      ...manifests,
      cargoLock: manifests.cargoLock.replace(
        'name = "pet-bridge"\nversion = "0.1.0"',
        'name = "pet-bridge"\nversion = "0.2.0"',
      ),
    };
    assert.throws(
      () => getConsistentVersion(inconsistent),
      /Cargo\.lock\(pet-bridge\)=0\.2\.0/,
    );
  });

  it("更新全部版本源的版本字段", () => {
    const updated = updateVersionContents(manifests, "0.1.1");
    assert.match(updated.packageJson, /"version": "0\.1\.1"/);
    assert.match(updated.desktopPackageJson, /"version": "0\.1\.1"/);
    assert.match(updated.harnessPackageJson, /"version": "0\.1\.1"/);
    assert.match(updated.tauriConfig, /"version": "0\.1\.1"/);
    assert.match(updated.desktopCargoToml, /version = "0\.1\.1"/);
    assert.match(updated.coreCargoToml, /version = "0\.1\.1"/);
    assert.match(updated.bridgeCargoToml, /version = "0\.1\.1"/);
    assert.equal(updated.cargoLock.match(/version = "0\.1\.1"/g)?.length, 3);
    assert.equal(getConsistentVersion(updated), "0.1.1");
  });

  it("Cargo.lock 存在同名远程依赖时只更新本地工作区包", () => {
    const cargoLock = `[[package]]
name = "pet-core"
version = "9.9.9"
source = "registry+https://github.com/rust-lang/crates.io-index"

${manifests.cargoLock}`;

    const updated = updateVersionContents({ ...manifests, cargoLock }, "0.1.1");
    assert.match(updated.cargoLock, /version = "9\.9\.9"/);
    assert.equal(updated.cargoLock.match(/version = "0\.1\.1"/g)?.length, 3);
    assert.equal(getConsistentVersion(updated), "0.1.1");
  });

  it("只更新顶层 JSON 版本字段", () => {
    const nestedVersionFirst = {
      ...manifests,
      packageJson:
        '{\n  "metadata": {\n    "version": "9.9.9"\n  },\n  "version": "0.1.0"\n}\n',
    };

    assert.equal(
      updateVersionContents(nestedVersionFirst, "0.1.1").packageJson,
      '{\n  "metadata": {\n    "version": "9.9.9"\n  },\n  "version": "0.1.1"\n}\n',
    );
  });
});

const ROOT = path.resolve("/repo");
const filePath = (relativePath) => path.join(ROOT, relativePath);

function releaseHarness(
  args,
  {
    branch = "main",
    failOn,
    localTag = "",
    remoteTag = "",
    runtime = { platform: "linux" },
    state = {},
    status = "",
    syncResults = ["0\t0"],
  } = {},
) {
  const files = new Map(
    Object.entries(VERSION_PATHS).map(([key, relativePath]) => [
      filePath(relativePath),
      manifests[key],
    ]),
  );
  const calls = [];
  const events = [];
  let syncResultIndex = 0;
  state.files = files;
  state.calls = calls;
  state.events = events;
  const fileSystem = {
    readFileSync: (targetPath) => files.get(targetPath),
    writeFileSync: (targetPath, value) => {
      events.push(["write", targetPath]);
      files.set(targetPath, value);
    },
  };
  const execute = (command, commandArgs) => {
    calls.push([command, ...commandArgs]);
    events.push(["command", command, ...commandArgs]);
    const key = [command, ...commandArgs].join(" ");
    if (key === failOn) throw new Error(`模拟失败：${key}`);
    if (key === "git status --porcelain") return status;
    if (key === "git branch --show-current") return branch;
    if (key === "git remote get-url origin") {
      return "git@github.com:owner/codex-pet.git";
    }
    if (key === `git rev-list --left-right --count HEAD...origin/${branch}`) {
      const result =
        syncResults[Math.min(syncResultIndex, syncResults.length - 1)];
      syncResultIndex += 1;
      return result;
    }
    if (key === "git tag --list v0.1.1") return localTag;
    if (key === "git ls-remote --tags origin refs/tags/v0.1.1") return remoteTag;
    return "";
  };
  const result = runRelease({
    args,
    cwd: ROOT,
    execute,
    fileSystem,
    output: { log() {}, error() {} },
    runtime,
  });
  return { calls, events, files, result };
}

describe("发布编排", () => {
  it("通过成功返回空分支名的查询给出 detached HEAD 专用提示", () => {
    const state = {};
    assert.throws(
      () => releaseHarness(["--current"], { branch: "", state }),
      /detached HEAD/,
    );
    assert.deepEqual(state.calls[1], ["git", "branch", "--show-current"]);
  });

  for (const [scenario, message, options] of [
    ["脏工作区", "工作区不干净", { status: " M package.json" }],
    [
      "缺失 origin",
      "模拟失败：git remote get-url origin",
      { failOn: "git remote get-url origin" },
    ],
    ["本地领先", "未完全同步", { syncResults: ["1\t0"] }],
    ["远端领先", "未完全同步", { syncResults: ["0\t1"] }],
    ["分叉", "未完全同步", { syncResults: ["2\t3"] }],
  ]) {
    it(`危险预检拒绝${scenario}且不产生发布副作用`, () => {
      const state = {};
      assert.throws(
        () => releaseHarness([], { ...options, state }),
        new RegExp(message),
      );
      assert.equal(
        state.calls.some(([command]) => command === "pnpm" || command === "cargo"),
        false,
      );
      assert.equal(state.events.some(([type]) => type === "write"), false);
    });
  }

  it("Windows 通过 Node 和 npm_execpath 启动 pnpm 检查", () => {
    const runtime = {
      platform: "win32",
      nodePath: "C:\\Program Files\\nodejs\\node.exe",
      npmExecPath: "C:\\Users\\dev\\AppData\\Local\\pnpm\\pnpm.cjs",
    };
    const { calls } = releaseHarness([], { runtime });
    assert.deepEqual(
      calls.find((call) => call.includes("test") && !call.includes("test:release")),
      [runtime.nodePath, runtime.npmExecPath, "test"],
    );
    assert.deepEqual(calls.find((call) => call.includes("build")), [
      runtime.nodePath,
      runtime.npmExecPath,
      "run",
      "build",
    ]);
  });

  it("Windows 支持直接执行 pnpm.exe 并拒绝无效入口", () => {
    assert.deepEqual(
      resolvePnpmCommand(["test"], {
        platform: "win32",
        nodePath: "C:\\nodejs\\node.exe",
        npmExecPath: "C:\\pnpm\\pnpm.exe",
      }),
      { command: "C:\\pnpm\\pnpm.exe", args: ["test"] },
    );
    assert.throws(
      () =>
        resolvePnpmCommand(["test"], {
          platform: "win32",
          nodePath: "C:\\nodejs\\node.exe",
          npmExecPath: "",
        }),
      /缺少 npm_execpath/,
    );
    assert.throws(
      () =>
        resolvePnpmCommand(["test"], {
          platform: "win32",
          nodePath: "C:\\nodejs\\node.exe",
          npmExecPath: "C:\\pnpm\\pnpm.cmd",
        }),
      /不能安全执行/,
    );
  });

  it("非 Windows 保留直接 pnpm 参数数组调用", () => {
    const { calls } = releaseHarness([], {
      runtime: {
        platform: "darwin",
        nodePath: "/opt/homebrew/bin/node",
        npmExecPath: "/opt/homebrew/lib/node_modules/pnpm/bin/pnpm.cjs",
      },
    });
    assert.deepEqual(
      calls.find((call) => call[0] === "pnpm" && call[1] === "test"),
      ["pnpm", "test"],
    );
    assert.deepEqual(
      calls.find(
        (call) => call[0] === "pnpm" && call[1] === "run" && call[2] === "build",
      ),
      ["pnpm", "run", "build"],
    );
  });

  it("校验、提交并推送新版本和标签", () => {
    const { calls, result } = releaseHarness([]);
    assert.deepEqual(result, { mode: "next-patch", version: "0.1.1" });
    assert.deepEqual(
      calls.find((call) => call[0] === "pnpm" && call[1] === "test"),
      ["pnpm", "test"],
    );
    assert.deepEqual(
      calls.find(
        (call) =>
          call[0] === "pnpm" && call[1] === "run" && call[2] === "typecheck",
      ),
      ["pnpm", "run", "typecheck"],
    );
    assert.deepEqual(
      calls.find((call) => call[0] === "cargo" && call[1] === "fmt"),
      ["cargo", "fmt", "--all", "--", "--check"],
    );
    assert.deepEqual(
      calls.find((call) => call[0] === "cargo" && call[1] === "clippy"),
      [
        "cargo",
        "clippy",
        "--workspace",
        "--all-targets",
        "--locked",
        "--",
        "-D",
        "warnings",
      ],
    );
    assert.deepEqual(
      calls.find((call) => call[0] === "cargo" && call[1] === "test"),
      ["cargo", "test", "--workspace", "--locked"],
    );
    assert.deepEqual(
      calls.find((call) => call[0] === "git" && call[1] === "commit"),
      ["git", "commit", "-m", "发布：v0.1.1"],
    );
    assert.deepEqual(
      calls.find(
        (call) => call[0] === "git" && call[1] === "tag" && call[2] === "-a",
      ),
      ["git", "tag", "-a", "v0.1.1", "-m", "发布 v0.1.1"],
    );
    assert.ok(
      calls.some(
        (call) =>
          call[0] === "git" &&
          call[1] === "push" &&
          call[2] === "origin" &&
          call[3] === "refs/tags/v0.1.1",
      ),
    );
  });

  it("current 不写版本或提交并强推当前标签", () => {
    const { calls, events, files, result } = releaseHarness(["--current"]);
    assert.deepEqual(result, { mode: "current", version: "0.1.0" });
    assert.equal(files.get(filePath("package.json")), manifests.packageJson);
    assert.equal(events.some(([type]) => type === "write"), false);
    assert.equal(calls.some((call) => call.includes("commit")), false);
    assert.deepEqual(
      calls.find((call) => call[0] === "git" && call[1] === "tag"),
      ["git", "tag", "-f", "-a", "v0.1.0", "-m", "发布 v0.1.0"],
    );
  });

  it("提交失败时撤销暂存并恢复版本文件", () => {
    const state = {};
    assert.throws(
      () => releaseHarness([], { failOn: "git commit -m 发布：v0.1.1", state }),
      /模拟失败/,
    );
    assert.ok(
      state.calls.some(
        (call) =>
          call[0] === "git" &&
          call[1] === "restore" &&
          call.includes("package.json"),
      ),
    );
    assert.equal(
      state.files.get(filePath("package.json")),
      manifests.packageJson,
    );
    assert.equal(
      state.files.get(filePath("Cargo.lock")),
      manifests.cargoLock,
    );
  });

  it("分支推送失败时给出 current 恢复命令", () => {
    assert.throws(
      () => releaseHarness([], { failOn: "git push origin main" }),
      /pnpm release --current/,
    );
  });

  it("标签推送失败时给出 current 重试命令", () => {
    assert.throws(
      () => releaseHarness([], { failOn: "git push origin refs/tags/v0.1.1" }),
      /pnpm release --current/,
    );
  });

  it("创建标签失败时给出 current 重试命令", () => {
    assert.throws(
      () =>
        releaseHarness([], { failOn: "git tag -a v0.1.1 -m 发布 v0.1.1" }),
      /pnpm release --current/,
    );
  });

  it("git add 部分失败时撤销版本文件暂存并恢复原始内容", () => {
    const state = {};
    const addCommand = `git add -- ${Object.values(VERSION_PATHS).join(" ")}`;
    assert.throws(
      () => releaseHarness([], { failOn: addCommand, state }),
      /模拟失败/,
    );
    assert.ok(
      state.calls.some((call) => call[0] === "git" && call[1] === "restore"),
    );
    assert.equal(
      state.files.get(filePath("package.json")),
      manifests.packageJson,
    );
    assert.equal(
      state.files.get(filePath("Cargo.lock")),
      manifests.cargoLock,
    );
  });

  for (const command of [
    "pnpm run test:release",
    "pnpm test",
    "pnpm run typecheck",
    "pnpm run build",
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --locked -- -D warnings",
    "cargo test --workspace --locked",
  ]) {
    it(`${command} 失败时不改版本或推送`, () => {
      const state = {};
      assert.throws(
        () => releaseHarness([], { failOn: command, state }),
        /模拟失败/,
      );
      assert.equal(state.events.some(([type]) => type === "write"), false);
      assert.equal(
        state.calls.some(
          (call) =>
            call[0] === "git" && ["add", "commit", "push"].includes(call[1]),
        ),
        false,
      );
    });
  }

  it("current 校验期间远端前进时拒绝改写标签", () => {
    const state = {};
    assert.throws(
      () =>
        releaseHarness(["--current"], {
          syncResults: ["0\t0", "0\t1"],
          state,
        }),
      /未完全同步/,
    );
    assert.equal(
      state.calls.some(
        (call) => call[0] === "git" && ["tag", "push"].includes(call[1]),
      ),
      false,
    );
  });

  it("目标 Tag 已存在时拒绝且无发布副作用", () => {
    const state = {};
    assert.throws(
      () => releaseHarness([], { localTag: "v0.1.1", state }),
      /标签 v0\.1\.1 已存在/,
    );
    assert.equal(
      state.calls.some(([command]) => command === "pnpm" || command === "cargo"),
      false,
    );
    assert.equal(state.events.some(([type]) => type === "write"), false);
  });
});
