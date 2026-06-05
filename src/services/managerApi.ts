import { invoke } from "@tauri-apps/api/core";

import type {
  HealthReport,
  ManagerSnapshot,
  OperationPlan,
  PayloadUpdateCheck,
} from "../shared/types";

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

const fallbackSnapshot: ManagerSnapshot = {
  managerVersion: "0.1.0",
  target: {
    os: "windows",
    arch: "x64",
    label: "Windows / X64",
  },
  settings: {
    mirrorBaseUrl: "https://codexapp.agentsmirror.com",
    installRoot: "%LOCALAPPDATA%\\Programs\\Codex",
    preserveUserDataByDefault: true,
  },
  endpoints: {
    manifestUrl: "https://codexapp.agentsmirror.com/latest/manifest",
    checksumsUrl: "https://codexapp.agentsmirror.com/latest/checksums",
    windowsMsixUrl: "https://codexapp.agentsmirror.com/latest/win",
    windowsUnpackedUrl: "https://codexapp.agentsmirror.com/latest/win-unpacked",
    macArm64Url: "https://codexapp.agentsmirror.com/latest/mac-arm64",
    macIntelUrl: "https://codexapp.agentsmirror.com/latest/mac-intel",
  },
  installation: {
    status: "not-detected",
    installRoot: "%LOCALAPPDATA%\\Programs\\Codex",
    detectedVersion: null,
    managedByThisApp: false,
  },
  availableActions: ["install", "update", "uninstall"],
};

const fallbackPlan: OperationPlan = {
  kind: "install",
  strategy: "windows-msix-preferred",
  installRoot: fallbackSnapshot.settings.installRoot,
  steps: [
    {
      id: "download-msix",
      title: "Download official MSIX",
      detail: fallbackSnapshot.endpoints.windowsMsixUrl,
      status: "ready",
    },
    {
      id: "verify-msix",
      title: "Verify package hash and identity",
      detail: "Compare SHA256SUMS.txt and AppxManifest.xml before install.",
      status: "ready",
    },
    {
      id: "install-msix",
      title: "Install via Windows package path",
      detail: "Prefer App Installer/MSIX; fixed-path unpacked install remains fallback.",
      status: "pending",
    },
  ],
};

function hasTauriRuntime() {
  return typeof window !== "undefined" && Boolean(window.__TAURI_INTERNALS__);
}

async function callCommand<T>(command: string, fallback: T): Promise<T> {
  if (!hasTauriRuntime()) {
    return fallback;
  }

  return invoke<T>(command);
}

export const managerApi = {
  getSnapshot() {
    return callCommand<ManagerSnapshot>("get_app_snapshot", fallbackSnapshot);
  },
  planInstall() {
    return callCommand<OperationPlan>("plan_install", fallbackPlan);
  },
  planUninstall() {
    return callCommand<OperationPlan>("plan_uninstall", {
      ...fallbackPlan,
      kind: "uninstall",
      steps: fallbackPlan.steps.map((step) => ({ ...step, status: "pending" })),
    });
  },
  checkUpdates() {
    return callCommand<PayloadUpdateCheck>("check_payload_updates", {
      status: "ready-to-check",
      manifestUrl: fallbackSnapshot.endpoints.manifestUrl,
      message: "Manifest client boundary is ready; network fetch and signature policy come next.",
    });
  },
  runHealthCheck() {
    return callCommand<HealthReport>("run_health_check", {
      checks: [
        {
          id: "platform",
          label: "Platform adapter",
          status: "ok",
          detail: fallbackSnapshot.target.label,
        },
        {
          id: "install-root",
          label: "Install root",
          status: "ok",
          detail: fallbackSnapshot.settings.installRoot,
        },
        {
          id: "manifest",
          label: "Mirror manifest",
          status: "ok",
          detail: fallbackSnapshot.endpoints.manifestUrl,
        },
      ],
    });
  },
};

