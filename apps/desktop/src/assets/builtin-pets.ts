import defaultAtlasUrl from '../../../../pet-output/yueying-cape-dancer/package/spritesheet.webp?url';
import whaleMaidAtlasUrl from '../../../../pet-output/deepseek-whale-maid/package/spritesheet.webp?url';
import type { PetAsset, PetManifest } from '../types';
import defaultManifest from './yueying.json';
import whaleMaidManifest from './deepseek-whale-maid.json';

function builtinAsset(manifest: Omit<PetManifest, 'version'> & { version: number }, atlasUrl: string): PetAsset {
  if (manifest.version !== 1) throw new Error(`不支持此内置桌宠清单版本：${manifest.id}`);
  return { manifest: { ...manifest, version: manifest.version }, atlasUrl };
}

export const defaultPet = builtinAsset(defaultManifest, defaultAtlasUrl);
const whaleMaid = builtinAsset(whaleMaidManifest, whaleMaidAtlasUrl);

export const builtinPets: ReadonlyMap<string, PetAsset> = new Map([
  [defaultPet.manifest.id, defaultPet],
  [whaleMaid.manifest.id, whaleMaid],
]);
