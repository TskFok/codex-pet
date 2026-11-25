export interface FrameRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface AnimationFrame {
  rect: FrameRect;
  durationMs: number;
}

export interface Animation {
  frames: AnimationFrame[];
  loopStart?: number;
}

export interface Atlas {
  path: string;
  width: number;
  height: number;
  cellWidth: number;
  cellHeight: number;
}

export interface PetManifest {
  version: 1;
  id: string;
  name: string;
  atlas: Atlas;
  animations: Record<string, Animation>;
  lookDirections?: FrameRect[];
}

export type Source = 'codex' | 'deepseek' | 'antigravity';
export interface SessionKey {
  source: Source;
  installationId: string;
  sessionId: string;
}
export interface ProjectContext {
  name: string;
  directory?: string;
}
export type Reason = 'approval' | 'answer';
export type Outcome = 'completed' | 'interrupted' | 'failed' | 'unknown';
export interface PendingRequest {
  id: string;
  turnId?: string;
  reason: Reason;
  summary?: string;
  lifetime?: 'turn' | 'until_resolved';
}
export interface SessionProjection {
  key: SessionKey;
  status: 'running' | 'needs_attention' | 'ended' | 'interrupted' | 'failed' | 'unknown';
  parentSessionId?: string;
  turnId?: string;
  pending: PendingRequest[];
  project?: ProjectContext;
  summary?: string;
  unread: boolean;
  lastEventAtMs: number;
}
export type NotificationKind =
  | {
      type: 'attention';
      requestId: string;
      reason: Reason;
      resolved: boolean;
      lifetime?: 'turn' | 'until_resolved';
    }
  | { type: 'turn_ended'; outcome: Outcome };
export interface NotificationRecord {
  id: string;
  key: SessionKey;
  turnId?: string;
  kind: NotificationKind;
  summary?: string;
  project?: ProjectContext;
  occurredAtMs: number;
  read: boolean;
}
export interface PetSnapshot {
  revision: number;
  sessions: SessionProjection[];
  notifications: NotificationRecord[];
  importedUnreadEnds?: SessionKey[];
  active?: SessionKey;
}
export interface Placement {
  x: number;
  y: number;
  scale: number;
  monitorId: string | null;
}
export type DeepSeekModel = 'deepseek-chat' | 'deepseek-reasoner';
export interface DeepSeekActivityStatus {
  configured: boolean;
  enabled: boolean;
  running: boolean;
  model: string;
  lastAction?: string | null;
  lastThought?: string | null;
  lastUpdatedAtMs?: number | null;
  lastError?: string | null;
}
export interface DeepSeekTestResult {
  ok: boolean;
  model: string;
  latencyMs: number;
  reply: string;
}
export interface PetActivityEvent {
  id: string;
  phase: 'start' | 'update' | 'end';
  state?: string;
  effect?: {
    x: number;
    y: number;
    kind: 'heart' | 'sparkle' | 'note' | 'star' | 'snack' | 'dizzy';
    bounce?: number;
    count?: number;
  };
  thought?: string;
  thoughtDurationMs?: number;
}
export interface Settings {
  revision: number;
  fps: 30 | 60;
  sound: boolean;
  autostart: boolean;
  petId: string;
  mutedSources: Source[];
  deepseekEnabled?: boolean;
  deepseekModel?: DeepSeekModel;
  deepseekApiKeyConfigured?: boolean;
  deepseekApiKeyHint?: string | null;
  placement: Placement;
}
export type SettingsPatch = Partial<
  Omit<Settings, 'revision' | 'placement' | 'deepseekApiKeyConfigured' | 'deepseekApiKeyHint'>
> & {
  deepseekApiKey?: string;
  placement?: Placement | Pick<Placement, 'scale'>;
};
export interface PetChoice {
  id: string;
  name: string;
}
export interface PetAsset {
  manifest: PetManifest;
  atlasUrl: string;
}
export interface IntegrationStatus {
  source: Source;
  host: string;
  installationId?: string | null;
  version?: string | null;
  installed: boolean;
  capabilities: { name: string; status: 'verified' | 'unsupported' | 'unverified' }[];
  problem?: string | null;
  target: string;
}
export interface InstallReport {
  changed: boolean;
  backupPath?: string | null;
  ownedEntryIds: string[];
  warnings: string[];
  bundlePath?: string | null;
}

export type OpenIntent = 'session' | 'application';
export type OpenOutcome =
  | { status: 'requested' }
  | { status: 'application_only'; opened: boolean }
  | { status: 'unavailable' };
