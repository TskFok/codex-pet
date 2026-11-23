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

export type Source = 'codex' | 'deepseek';
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
export interface Settings {
  revision: number;
  fps: 30 | 60;
  sound: boolean;
  autostart: boolean;
  petId: string;
  mutedSources: Source[];
  placement: Placement;
}
export type SettingsPatch = Partial<Omit<Settings, 'revision' | 'placement'>> & {
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
