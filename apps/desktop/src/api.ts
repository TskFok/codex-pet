import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type {
  DeepSeekActivityStatus,
  DeepSeekModel,
  DeepSeekTestResult,
  InstallReport,
  IntegrationStatus,
  OpenIntent,
  OpenOutcome,
  PetAsset,
  PetChoice,
  PetManifest,
  PetSnapshot,
  PetSoundCue,
  SessionKey,
  Settings,
  SettingsPatch,
  SoundPack,
  Source,
} from './types';

type Subscribe<T> = (apply: (value: T) => void) => Promise<() => void>;

/** 初值和事件走同一个修订门。初始请求失败时，后台服务稍后的广播仍能恢复。 */
export async function observeSnapshot<T extends { revision: number }>(
  subscribe: Subscribe<T>,
  get: () => Promise<T>,
  render: (value: T) => void,
  error?: (e: unknown) => void,
): Promise<() => void> {
  let latest = -1;
  const apply = (value: T) => {
    if (value.revision <= latest) return;
    latest = value.revision;
    render(value);
  };
  const stop = await subscribe(apply);
  try {
    apply(await get());
  } catch (e) {
    if (latest < 0 && !error) {
      stop();
      throw e;
    }
    if (latest < 0) error?.(e);
  }
  return stop;
}

function event<T>(name: string): Subscribe<T> {
  return (apply) => listen<T>(name, (e) => apply(e.payload));
}
export const api = {
  snapshot: (): Promise<PetSnapshot> => invoke('get_snapshot'),
  snapshots: event<PetSnapshot>('pet:snapshot'),
  markRead: (key: SessionKey): Promise<PetSnapshot> => invoke('mark_read', { key }),
  markReadMany: (keys: SessionKey[]): Promise<PetSnapshot> => invoke('mark_read_many', { keys }),
  clear: (): Promise<PetSnapshot> => invoke('clear_history'),
  openSession: (key: SessionKey, intent: OpenIntent): Promise<OpenOutcome> =>
    invoke('open_session', { key, intent }),
  openNotifications: (): Promise<void> => invoke('open_notifications'),
  copySession: (key: SessionKey): Promise<void> => invoke('copy_session_id', { key }),
  settings: (): Promise<Settings> => invoke('get_settings'),
  settingsEvents: event<Settings>('pet:settings'),
  update: (patch: SettingsPatch): Promise<Settings> => invoke('update_settings', { patch }),
  deepseekActivityStatus: (): Promise<DeepSeekActivityStatus> => invoke('get_deepseek_activity_status'),
  deepseekActivityStatusEvents: event<DeepSeekActivityStatus>('pet:activity-status'),
  testDeepSeekConnection: (apiKey?: string, model?: DeepSeekModel): Promise<DeepSeekTestResult> =>
    invoke('test_deepseek_connection', { apiKey, model }),
  triggerPetActivity: (): Promise<void> => invoke('trigger_pet_activity'),
  interruptPetActivity: (): Promise<void> => invoke('interrupt_pet_activity'),
  playAlert: (source: Source, cue?: PetSoundCue): Promise<void> =>
    invoke('play_alert', { source, cue }),
  playPetSound: (cue: PetSoundCue): Promise<void> => invoke('play_pet_sound', { cue }),
  previewPetSound: (options: {
    cue: PetSoundCue;
    soundPack?: SoundPack;
    soundVolume?: number;
    petId?: string;
  }): Promise<void> => invoke('preview_pet_sound', options),
  pets: (): Promise<PetChoice[]> => invoke('list_pets'),
  chooseDirectory: (): Promise<string | null> => invoke('choose_pet_directory'),
  importPet: (path: string): Promise<PetManifest> => invoke('import_pet_package', { path }),
  asset: (id: string): Promise<PetAsset> => invoke('get_pet_asset', { id }),
  integrations: (): Promise<IntegrationStatus[]> => invoke('integration_status'),
  integrate: (source: Source, action: 'install' | 'remove'): Promise<InstallReport> =>
    invoke('apply_integration', { source, action }),
};
export function message(error: unknown): string {
  if (typeof error === 'object' && error && 'message' in error) return String(error.message);
  return String(error);
}
