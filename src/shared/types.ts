export type OperatingSystem = "windows" | "macos" | "linux" | "unknown";
export type Architecture = "x64" | "arm64" | "unknown";

export interface InstalledCodex {
  path: string;
  build: number;
  arch: string;
}

export interface UpdateStrategy {
  kind: "delta" | "full";
  fromBuild?: number;
}

export interface UpdatePlan {
  upToDate: boolean;
  currentBuild: number;
  latestBuild: number;
  latestShortVersion: string;
  strategy: UpdateStrategy;
  downloadUrl: string;
  downloadSize: number;
  edSignature: string | null;
  fullSize: number;
  savingsPct: number;
}

export interface MacUpdateReport {
  appcastUrl: string;
  installed: InstalledCodex | null;
  simulatedBuild: number | null;
  plan: UpdatePlan | null;
}

export interface MacStageReport {
  upToDate: boolean;
  strategy: string;
  latestBuild: number;
  latestShortVersion: string;
  downloadSize: number;
  fullSize: number;
  savingsPct: number;
  stagedPath: string | null;
  verified: boolean;
}

export type InstallClass = "managed" | "external" | "none";

export interface MacInstallStatus {
  installed: InstalledCodex | null;
  status: InstallClass;
}
