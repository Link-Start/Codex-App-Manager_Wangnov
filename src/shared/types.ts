export type OperatingSystem = "windows" | "macos" | "linux" | "unknown";
export type Architecture = "x64" | "arm64" | "unknown";

export interface Target {
  os: OperatingSystem;
  arch: Architecture;
  label: string;
}

export interface AppSettings {
  mirrorBaseUrl: string;
  installRoot: string;
  preserveUserDataByDefault: boolean;
}

export interface MirrorEndpoints {
  manifestUrl: string;
  checksumsUrl: string;
  windowsMsixUrl: string;
  windowsUnpackedUrl: string;
  macArm64Url: string;
  macIntelUrl: string;
}

export type InstallationStatus = "not-detected" | "managed" | "external" | "unknown";

export interface ManagedInstallation {
  status: InstallationStatus;
  installRoot: string;
  detectedVersion: string | null;
  managedByThisApp: boolean;
}

export interface ManagerSnapshot {
  managerVersion: string;
  target: Target;
  settings: AppSettings;
  endpoints: MirrorEndpoints;
  installation: ManagedInstallation;
  availableActions: string[];
}

export type OperationKind = "install" | "update" | "uninstall";
export type OperationStrategy =
  | "windows-msix-preferred"
  | "windows-fixed-path-unpacked"
  | "macos-dmg-replace"
  | "managed-uninstall"
  | "unsupported";
export type OperationStepStatus = "ready" | "pending" | "blocked";

export interface OperationStep {
  id: string;
  title: string;
  detail: string;
  status: OperationStepStatus;
}

export interface OperationPlan {
  kind: OperationKind;
  strategy: OperationStrategy;
  installRoot: string;
  steps: OperationStep[];
}

export type PayloadUpdateStatus =
  | "ready-to-check"
  | "checking"
  | "update-available"
  | "current"
  | "blocked";

export interface PayloadUpdateCheck {
  status: PayloadUpdateStatus;
  manifestUrl: string;
  message: string;
}

export type HealthStatus = "ok" | "warning" | "blocked";

export interface HealthCheck {
  id: string;
  label: string;
  status: HealthStatus;
  detail: string;
}

export interface HealthReport {
  checks: HealthCheck[];
}
