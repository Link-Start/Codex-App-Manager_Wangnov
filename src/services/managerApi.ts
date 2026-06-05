import { invoke } from "@tauri-apps/api/core";
import { relaunch } from "@tauri-apps/plugin-process";
import { check } from "@tauri-apps/plugin-updater";

import type { MacInstallStatus, MacStageReport, MacUpdateReport } from "../shared/types";

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

function hasTauriRuntime(): boolean {
  return typeof window !== "undefined" && Boolean(window.__TAURI_INTERNALS__);
}

// Browser-dev fallbacks (no Tauri runtime) — a simulated "one version behind"
// so the UI renders meaningfully outside the desktop shell.
const FALLBACK_PLAN: MacUpdateReport = {
  appcastUrl: "https://persistent.oaistatic.com/codex-app-prod/appcast.xml",
  installed: null,
  simulatedBuild: 3511,
  plan: {
    upToDate: false,
    currentBuild: 3511,
    latestBuild: 3575,
    latestShortVersion: "26.602.30954",
    strategy: { kind: "delta", fromBuild: 3511 },
    downloadUrl:
      "https://persistent.oaistatic.com/codex-app-prod/Codex3575-3511-arm64.delta",
    downloadSize: 18260894,
    edSignature: "(browser-dev mock)",
    fullSize: 406581087,
    savingsPct: 95.5,
  },
};

const FALLBACK_STAGE: MacStageReport = {
  upToDate: false,
  strategy: "delta-from-3511",
  latestBuild: 3575,
  latestShortVersion: "26.602.30954",
  downloadSize: 18260894,
  fullSize: 406581087,
  savingsPct: 95.5,
  stagedPath: "(browser-dev mock) …/Codex3575-3511-arm64.delta",
  verified: true,
};

export const managerApi = {
  macPlanUpdate(simulatedBuild?: number): Promise<MacUpdateReport> {
    if (!hasTauriRuntime()) {
      return Promise.resolve({ ...FALLBACK_PLAN, simulatedBuild: simulatedBuild ?? null });
    }
    return invoke<MacUpdateReport>("mac_plan_update", {
      simulatedBuild: simulatedBuild ?? null,
    });
  },
  macStageUpdate(simulatedBuild?: number): Promise<MacStageReport> {
    if (!hasTauriRuntime()) {
      return Promise.resolve(FALLBACK_STAGE);
    }
    return invoke<MacStageReport>("mac_stage_update", {
      simulatedBuild: simulatedBuild ?? null,
    });
  },
  // Self-update the manager itself via the Tauri updater (minisign-signed,
  // full bundle). Endpoints + signing are server-side (see roadmap §4).
  async checkManagerUpdate(): Promise<string> {
    if (!hasTauriRuntime()) {
      return "（浏览器开发态：manager 自更新在桌面 app 内可用）";
    }
    const update = await check();
    if (!update) {
      return "manager 已是最新版";
    }
    await update.downloadAndInstall();
    await relaunch();
    return `已安装 ${update.version}，正在重启…`;
  },
  macStatus(): Promise<MacInstallStatus> {
    if (!hasTauriRuntime()) {
      return Promise.resolve({ installed: null, status: "none" });
    }
    return invoke<MacInstallStatus>("mac_status");
  },
  macAdopt(): Promise<MacInstallStatus> {
    if (!hasTauriRuntime()) {
      return Promise.resolve({ installed: null, status: "managed" });
    }
    return invoke<MacInstallStatus>("mac_adopt");
  },
};
