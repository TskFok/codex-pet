import { createHash } from 'node:crypto';
import { open, readFile } from 'node:fs/promises';

export const SUPPORTED_TARGETS = {
  'aarch64-apple-darwin': {
    os: 'macos',
    arch: 'arm64',
    bridgeFileName: 'pet-bridge',
    expectedBinaryKind: 'macho-arm64',
  },
  'x86_64-apple-darwin': {
    os: 'macos',
    arch: 'x86_64',
    bridgeFileName: 'pet-bridge',
    expectedBinaryKind: 'macho-x86_64',
  },
  'x86_64-pc-windows-msvc': {
    os: 'windows',
    arch: 'x86_64',
    bridgeFileName: 'pet-bridge.exe',
    expectedBinaryKind: 'pe-x86_64',
  },
};

export function hostDefaultTarget() {
  if (process.platform === 'darwin') {
    return process.arch === 'arm64' ? 'aarch64-apple-darwin' : 'x86_64-apple-darwin';
  }
  if (process.platform === 'win32' && process.arch === 'x64') {
    return 'x86_64-pc-windows-msvc';
  }
  throw new Error(`当前主机平台 ${process.platform}/${process.arch} 不在首版目标范围内`);
}

export async function sha256File(filePath) {
  const bytes = await readFile(filePath);
  return createHash('sha256').update(bytes).digest('hex');
}

/**
 * 读取二进制头部，判断真实可执行文件架构（Mach-O arm64 / x86_64 或 Windows PE x86_64）。
 */
export async function detectBinaryKind(filePath) {
  const handle = await open(filePath, 'r');
  try {
    const header = Buffer.alloc(512);
    const { bytesRead } = await handle.read(header, 0, header.length, 0);
    if (bytesRead < 8) return 'unknown';

    const magicLE = header.readUInt32LE(0);
    const magicBE = header.readUInt32BE(0);

    // 64-bit Mach-O (MH_MAGIC_64 / MH_CIGAM_64)
    if (magicLE === 0xfeedfacf || magicBE === 0xfeedfacf) {
      const cpuType = magicLE === 0xfeedfacf ? header.readUInt32LE(4) : header.readUInt32BE(4);
      if (cpuType === 0x0100000c) return 'macho-arm64';
      if (cpuType === 0x01000007) return 'macho-x86_64';
      return `macho-other(0x${cpuType.toString(16)})`;
    }

    // Universal Mach-O Fat Binary (FAT_MAGIC = 0xcafebabe)
    if (magicBE === 0xcafebabe) {
      const count = header.readUInt32BE(4);
      const arches = [];
      for (let i = 0; i < count && 8 + (i + 1) * 20 <= bytesRead; i++) {
        const cpuType = header.readUInt32BE(8 + i * 20);
        if (cpuType === 0x0100000c) arches.push('arm64');
        else if (cpuType === 0x01000007) arches.push('x86_64');
      }
      if (arches.length > 0) return `macho-universal(${arches.sort().join('+')})`;
      return 'macho-universal-unknown';
    }

    // Windows PE ('MZ' + e_lfanew -> 'PE\0\0' + Machine)
    if (header[0] === 0x4d && header[1] === 0x5a && bytesRead >= 0x40) {
      const peOffset = header.readUInt32LE(0x3c);
      const peBuf = Buffer.alloc(6);
      const peRead = await handle.read(peBuf, 0, 6, peOffset);
      if (
        peRead.bytesRead === 6 &&
        peBuf[0] === 0x50 &&
        peBuf[1] === 0x45 &&
        peBuf[2] === 0x00 &&
        peBuf[3] === 0x00
      ) {
        const machine = peBuf.readUInt16LE(4);
        if (machine === 0x8664) return 'pe-x86_64';
        if (machine === 0xaa64) return 'pe-arm64';
        if (machine === 0x014c) return 'pe-x86';
        return `pe-other(0x${machine.toString(16)})`;
      }
    }

    return 'unknown';
  } finally {
    await handle.close();
  }
}

export function binaryMatchesTarget(detectedKind, targetTriple) {
  const spec = SUPPORTED_TARGETS[targetTriple];
  if (!spec) return false;
  return detectedKind === spec.expectedBinaryKind;
}
