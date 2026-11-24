const STABLE_SEMVER = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;

export const CARGO_LOCK_PACKAGES = ["pet-core", "pet-bridge", "pet-desktop"];

export const VERSION_PATHS = {
  packageJson: "package.json",
  desktopPackageJson: "apps/desktop/package.json",
  harnessPackageJson: "integrations/deepseek-harness/package.json",
  tauriConfig: "apps/desktop/src-tauri/tauri.conf.json",
  desktopCargoToml: "apps/desktop/src-tauri/Cargo.toml",
  coreCargoToml: "crates/pet-core/Cargo.toml",
  bridgeCargoToml: "crates/pet-bridge/Cargo.toml",
  cargoLock: "Cargo.lock",
};

const JSON_VERSION_KEYS = [
  "packageJson",
  "desktopPackageJson",
  "harnessPackageJson",
  "tauriConfig",
];

const CARGO_TOML_KEYS = [
  "desktopCargoToml",
  "coreCargoToml",
  "bridgeCargoToml",
];

function parseVersion(version) {
  const match = STABLE_SEMVER.exec(version);
  if (!match) {
    throw new Error(`版本 ${version} 不是稳定 SemVer（格式必须为 x.y.z）`);
  }
  return match.slice(1).map(BigInt);
}

function compareVersions(left, right) {
  const leftParts = parseVersion(left);
  const rightParts = parseVersion(right);
  for (let index = 0; index < leftParts.length; index += 1) {
    if (leftParts[index] !== rightParts[index]) {
      return leftParts[index] < rightParts[index] ? -1 : 1;
    }
  }
  return 0;
}

export function parseReleaseArgs(args) {
  const normalized = args[0] === "--" ? args.slice(1) : args;
  if (normalized.length === 0) return { mode: "next-patch" };
  if (normalized.length === 1 && normalized[0] === "--current") {
    return { mode: "current" };
  }
  if (normalized.includes("--current")) {
    throw new Error("--current 不能与其他参数组合");
  }
  if (normalized.length !== 1) {
    throw new Error("用法：pnpm release [x.y.z | --current]");
  }
  parseVersion(normalized[0]);
  return { mode: "explicit", version: normalized[0] };
}

export function resolveTargetVersion(request, current) {
  const [major, minor, patch] = parseVersion(current);
  if (request.mode === "current") return current;
  if (request.mode === "next-patch") return `${major}.${minor}.${patch + 1n}`;
  if (compareVersions(request.version, current) <= 0) {
    throw new Error(`目标版本 ${request.version} 必须高于当前版本 ${current}`);
  }
  return request.version;
}

function jsonVersion(content, filePath) {
  let parsed;
  try {
    parsed = JSON.parse(content);
  } catch (error) {
    throw new Error(
      `${filePath} JSON 解析失败：${error instanceof Error ? error.message : String(error)}`,
      { cause: error },
    );
  }
  if (parsed === null || Array.isArray(parsed) || typeof parsed !== "object") {
    throw new Error(`${filePath} JSON 根值必须是对象`);
  }
  const value = parsed.version;
  if (typeof value !== "string") {
    throw new Error(`${filePath} 缺少字符串 version`);
  }
  parseVersion(value);
  return value;
}

function cargoTomlPackageBlock(content, filePath) {
  const match = content.match(/\[package\][\s\S]*?(?=\n\[|$)/);
  if (!match) throw new Error(`${filePath} 缺少 [package]`);
  return match[0];
}

function cargoLockPackageBlock(content, packageName) {
  const blocks =
    content.match(/\[\[package\]\][\s\S]*?(?=\n\[\[package\]\]|$)/g) ?? [];
  const matches = blocks.filter(
    (block) =>
      new RegExp(`^name\\s*=\\s*"${packageName}"\\s*$`, "m").test(block) &&
      !/^source\s*=\s*"[^"]+"\s*$/m.test(block),
  );
  if (matches.length !== 1) {
    throw new Error(
      `${VERSION_PATHS.cargoLock} 必须包含一个无 source 的 ${packageName} 工作区包`,
    );
  }
  return matches[0];
}

function blockVersion(block, label) {
  const match = block.match(/^version\s*=\s*"([^"]+)"\s*$/m);
  if (!match) throw new Error(`${label} 缺少 package version`);
  parseVersion(match[1]);
  return match[1];
}

export function getConsistentVersion(contents) {
  const versions = [];
  for (const key of JSON_VERSION_KEYS) {
    const filePath = VERSION_PATHS[key];
    versions.push([filePath, jsonVersion(contents[key], filePath)]);
  }
  for (const key of CARGO_TOML_KEYS) {
    const filePath = VERSION_PATHS[key];
    const block = cargoTomlPackageBlock(contents[key], filePath);
    versions.push([filePath, blockVersion(block, filePath)]);
  }
  for (const packageName of CARGO_LOCK_PACKAGES) {
    const label = `${VERSION_PATHS.cargoLock}(${packageName})`;
    const block = cargoLockPackageBlock(contents.cargoLock, packageName);
    versions.push([label, blockVersion(block, label)]);
  }

  const unique = new Set(versions.map(([, value]) => value));
  if (unique.size !== 1) {
    const detail = versions
      .map(([label, value]) => `${label}=${value}`)
      .join("，");
    throw new Error(`版本不一致：${detail}`);
  }
  return versions[0][1];
}

function replaceJsonVersion(content, version, filePath) {
  jsonVersion(content, filePath);
  const versionPattern = /("version"\s*:\s*")[^"]+("\s*[,}])/g;
  let depth = 0;
  let inString = false;
  let escaped = false;
  let cursor = 0;

  for (const match of content.matchAll(versionPattern)) {
    while (cursor < match.index) {
      const character = content[cursor];
      cursor += 1;
      if (inString) {
        if (escaped) escaped = false;
        else if (character === "\\") escaped = true;
        else if (character === '"') inString = false;
      } else if (character === '"') inString = true;
      else if (character === "{" || character === "[") depth += 1;
      else if (character === "}" || character === "]") depth -= 1;
    }

    if (!inString && depth === 1) {
      return `${content.slice(0, match.index)}${match[1]}${version}${match[2]}${content.slice(
        match.index + match[0].length,
      )}`;
    }
  }
  throw new Error(`${filePath} 缺少字符串 version`);
}

function replaceCargoTomlVersion(content, version, filePath) {
  const block = cargoTomlPackageBlock(content, filePath);
  blockVersion(block, filePath);
  const updatedBlock = block.replace(
    /^(version\s*=\s*")[^"]+("\s*)$/m,
    `$1${version}$2`,
  );
  return content.replace(block, () => updatedBlock);
}

function replaceCargoLockVersion(content, version) {
  let updated = content;
  for (const packageName of CARGO_LOCK_PACKAGES) {
    const label = `${VERSION_PATHS.cargoLock}(${packageName})`;
    const block = cargoLockPackageBlock(updated, packageName);
    blockVersion(block, label);
    const updatedBlock = block.replace(
      /^(version\s*=\s*")[^"]+("\s*)$/m,
      `$1${version}$2`,
    );
    updated = updated.replace(block, () => updatedBlock);
  }
  return updated;
}

export function updateVersionContents(contents, version) {
  parseVersion(version);
  const updated = {};
  for (const key of JSON_VERSION_KEYS) {
    updated[key] = replaceJsonVersion(contents[key], version, VERSION_PATHS[key]);
  }
  for (const key of CARGO_TOML_KEYS) {
    updated[key] = replaceCargoTomlVersion(
      contents[key],
      version,
      VERSION_PATHS[key],
    );
  }
  updated.cargoLock = replaceCargoLockVersion(contents.cargoLock, version);
  return updated;
}
