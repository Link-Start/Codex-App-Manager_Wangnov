import { invoke } from "@tauri-apps/api/core";
import { relaunch } from "@tauri-apps/plugin-process";
import { check } from "@tauri-apps/plugin-updater";

import type {
  AppSettings,
  MacInstallStatus,
  MacPerformReport,
  MacStageReport,
  MacUninstallReport,
  MacUpdateReport,
} from "../shared/types";
import { DEFAULT_SETTINGS } from "../shared/types";

const SETTINGS_LS = "cam.settings";

function localSettings(): AppSettings {
  try {
    const raw = localStorage.getItem(SETTINGS_LS);
    if (raw) return { ...DEFAULT_SETTINGS, ...JSON.parse(raw), signedOnly: true };
  } catch {
    // ignore
  }
  return { ...DEFAULT_SETTINGS };
}

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
  // Destructive: reconstruct + codesign-gate + atomic swap + relaunch (or
  // rollback). `confirm` must be true; the backend rejects it otherwise. Guarded
  // by a UI second confirmation before this is ever called. The expected target
  // (from/to build + install path the user confirmed) is sent so the backend can
  // refuse if reality drifted (appcast refresh / Codex self-update) since confirm.
  macPerformUpdate(expected: {
    fromBuild: number;
    toBuild: number;
    path: string;
  }): Promise<MacPerformReport> {
    if (!hasTauriRuntime()) {
      return Promise.resolve({
        upToDate: false,
        fromBuild: expected.fromBuild,
        toBuild: expected.toBuild,
        strategy: "delta-from-3511",
        installedPath: expected.path,
        verified: true,
        relaunched: true,
        rolledBack: false,
        message: "（浏览器开发态：真实替换仅在桌面 app 内执行）",
      });
    }
    return invoke<MacPerformReport>("mac_perform_update", {
      confirm: true,
      expectedFromBuild: expected.fromBuild,
      expectedToBuild: expected.toBuild,
      expectedPath: expected.path,
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

  // Fresh-install the latest Codex (full package) into /Applications.
  macInstall(): Promise<MacInstallStatus> {
    if (!hasTauriRuntime()) {
      return Promise.resolve({
        installed: { path: "/Applications/Codex.app", build: 3575, arch: "arm64" },
        status: "managed",
      });
    }
    return invoke<MacInstallStatus>("mac_install");
  },

  // Settings (update source + general). The backend persists them so the source
  // choice actually drives which appcast the update flow reads.
  async getSettings(): Promise<AppSettings> {
    if (!hasTauriRuntime()) {
      return localSettings();
    }
    try {
      return await invoke<AppSettings>("get_settings");
    } catch {
      return localSettings();
    }
  },
  async setSettings(next: AppSettings): Promise<AppSettings> {
    const safe = { ...next, signedOnly: true };
    if (!hasTauriRuntime()) {
      localStorage.setItem(SETTINGS_LS, JSON.stringify(safe));
      return safe;
    }
    const saved = await invoke<AppSettings>("set_settings", { settings: safe });
    localStorage.setItem(SETTINGS_LS, JSON.stringify(saved));
    return saved;
  },

  // Destructive: remove the Codex app. keepCodexHome defaults to true so the
  // user's ~/.codex (sign-in, sessions, config) survives unless they opt out.
  macUninstall(keepCodexHome: boolean): Promise<MacUninstallReport> {
    if (!hasTauriRuntime()) {
      return Promise.resolve({
        removed: true,
        keptCodexHome: keepCodexHome,
        message: keepCodexHome ? "（浏览器开发态）已卸载,保留数据" : "（浏览器开发态）已卸载并清除数据",
      });
    }
    return invoke<MacUninstallReport>("mac_uninstall", { confirm: true, keepCodexHome });
  },
};
