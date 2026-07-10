import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { listen } from "@tauri-apps/api/event";

import { managerApi } from "../../services/managerApi";
import type {
  AppSettings,
  CapabilityCheck,
  DownloadProgress,
  InstalledWindowsCodex,
  OperationSnapshot,
  WinCapabilityReport,
  WindowsUpdatePlan,
  WinInstallStatus,
  WinPerformReport,
  WinUpdateReport,
} from "../../shared/types";
import { DEFAULT_SETTINGS, emptyOperationOutcome } from "../../shared/types";
import { I18nProvider } from "../i18n";
import { ThemeProvider } from "../theme";
import { WinHome } from "./WinHome";

vi.mock("../motion", () => ({ useHomeMotion: () => {} }));

vi.mock("../../services/managerApi", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../services/managerApi")>();
  return {
    ...actual,
    managerApi: {
      getSettings: vi.fn(),
      setSettings: vi.fn(),
      getOperationSnapshot: vi.fn(() => Promise.resolve(null)),
      winStatus: vi.fn(),
      winPlanUpdate: vi.fn(),
      winPerformUpdate: vi.fn(),
      winAdopt: vi.fn(),
      winAdoptPath: vi.fn(),
      winLaunch: vi.fn(),
      winPauseDownload: vi.fn(),
      winCancelDownload: vi.fn(),
      winDiscardDownload: vi.fn(),
      winPickExistingInstall: vi.fn(),
      winPickInstallDir: vi.fn(),
      winDefaultInstallRoot: vi.fn(),
    },
  };
});

const api = vi.mocked(managerApi);
const listenMock = vi.mocked(listen);

const ok: CapabilityCheck = { state: "available", detail: "" };
const unknown: CapabilityCheck = { state: "unknown", detail: "" };

const CAPS_OK: WinCapabilityReport = {
  addAppxPackage: ok,
  appxService: ok,
  sideloadPolicy: ok,
  appInstaller: ok,
  msixDeployment: ok,
  meteredNetwork: ok,
  recommendation: "msix-preferred",
  notes: [],
};

const INSTALLED: InstalledWindowsCodex = {
  path: "C:\\Program Files\\WindowsApps\\Codex",
  version: "1.0.0",
  arch: "x64",
  source: "msix",
  packageFamilyName: "OpenAI.Codex_x",
};

const PLAN_UPDATE: WindowsUpdatePlan = {
  upToDate: false,
  currentVersion: "1.0.0",
  latestVersion: "2.0.0",
  packageMoniker: "Codex_2.0.0_x64",
  packageUrl: "https://example.invalid/codex.msix",
  downloadSize: 2048,
  sha256: "deadbeef",
  route: "msix-sideload",
  portableFallbackReady: false,
  warnings: [],
};

function report(overrides: Partial<WinUpdateReport> = {}): WinUpdateReport {
  return {
    manifestUrl: "m",
    checksumsUrl: "c",
    packageUrl: "p",
    release: {
      version: "2.0.0",
      packageVersion: "2.0.0.0",
      packageMoniker: "Codex_2.0.0_x64",
      architecture: "x64",
      contentLength: 2048,
      etag: null,
      storeProductId: null,
      packageIdentity: null,
    },
    installed: INSTALLED,
    capabilities: CAPS_OK,
    plan: PLAN_UPDATE,
    ...overrides,
  };
}

const STATUS_MANAGED: WinInstallStatus = { installed: INSTALLED, status: "managed" };
const ACTIVE_OPERATION: OperationSnapshot = {
  id: "op-active",
  kind: "update",
  phase: "downloading",
  progress: { downloaded: 10, total: 100, source: "mirror.example" },
  paused: false,
  cancellable: true,
  interruptible: true,
};

const PERFORM_OK: WinPerformReport = {
  success: true,
  action: "msix-sideload",
  message: "updated",
  stage: {
    upToDate: false,
    route: "msix-sideload",
    latestVersion: "2.0.0",
    packageMoniker: "Codex_2.0.0_x64",
    downloadSize: 2048,
    stagedPath: "s",
    sha256: "deadbeef",
    hashVerified: true,
    authenticode: null,
    identity: null,
    identityVerified: true,
    installReady: true,
    portableFallbackReady: false,
    notes: [],
  },
  sideload: null,
  portable: null,
  msixHealth: null,
  installed: INSTALLED,
  fallbackAvailable: false,
  fallbackAttempted: false,
  notes: [],
  outcome: emptyOperationOutcome({
    primaryOk: true,
    appState: "present",
    installClass: "managed",
  }),
};

function settings(overrides: Partial<AppSettings> = {}): AppSettings {
  return { ...DEFAULT_SETTINGS, ...overrides };
}

function renderWinHome() {
  return render(
    <ThemeProvider>
      <I18nProvider>
        <WinHome onOpenSettings={vi.fn()} />
      </I18nProvider>
    </ThemeProvider>,
  );
}

describe("WinHome state machine", () => {
  beforeEach(() => {
    localStorage.setItem("cam.lang", "zh-CN");
    api.getSettings.mockResolvedValue(settings());
    api.winDefaultInstallRoot.mockResolvedValue(DEFAULT_SETTINGS.installRoot);
    api.winStatus.mockResolvedValue(STATUS_MANAGED);
    api.winPlanUpdate.mockResolvedValue(report());
    api.winPerformUpdate.mockResolvedValue(PERFORM_OK);
    api.winPauseDownload.mockResolvedValue(true);
    api.winCancelDownload.mockResolvedValue(true);
    api.winDiscardDownload.mockResolvedValue(undefined);
    api.getOperationSnapshot.mockResolvedValue(null);
  });

  it("offers install when nothing is detected", async () => {
    api.getSettings.mockResolvedValue(settings({ checkOnStartup: false }));
    api.winStatus.mockResolvedValue({ installed: null, status: "none" });
    renderWinHome();
    expect(await screen.findByRole("button", { name: /安装 Codex/ })).toBeInTheDocument();
  });

  it("signs the perform expectation from the SAME report snapshot it shows", async () => {
    const user = userEvent.setup();
    renderWinHome();
    await user.click(await screen.findByRole("button", { name: /立即更新/ }));
    await user.click(await screen.findByRole("button", { name: "更新" }));
    await waitFor(() =>
      expect(api.winPerformUpdate).toHaveBeenCalledWith(
        true,
        {
          currentVersion: "1.0.0",
          latestVersion: "2.0.0",
          packageMoniker: "Codex_2.0.0_x64",
          route: "msix-sideload",
        },
        undefined,
      ),
    );
  });

  it("settles on up-to-date", async () => {
    api.winPlanUpdate.mockResolvedValue(
      report({ plan: { ...PLAN_UPDATE, upToDate: true, latestVersion: "1.0.0" } }),
    );
    renderWinHome();
    expect(
      await screen.findByText("已是最新", { selector: ".headline" }),
    ).toBeInTheDocument();
  });

  it("gates an external install behind adopt", async () => {
    api.getSettings.mockResolvedValue(settings({ checkOnStartup: false }));
    api.winStatus.mockResolvedValue({ installed: INSTALLED, status: "external" });
    api.winAdopt.mockResolvedValue(STATUS_MANAGED);
    const user = userEvent.setup();
    renderWinHome();
    await user.click(await screen.findByRole("button", { name: /开始管理/ }));
    await waitFor(() => expect(api.winAdopt).toHaveBeenCalledTimes(1));
  });

  it("treats a stale expectation as a notice and re-checks", async () => {
    const user = userEvent.setup();
    api.getSettings.mockResolvedValue(settings({ askBefore: false }));
    api.winPerformUpdate.mockRejectedValue({ code: "stale_expectation", message: "stale" });
    api.winPlanUpdate
      .mockResolvedValueOnce(report())
      .mockResolvedValue(
        report({ plan: { ...PLAN_UPDATE, upToDate: true, latestVersion: "1.0.0" } }),
      );
    renderWinHome();
    await user.click(await screen.findByRole("button", { name: /立即更新/ }));
    expect(await screen.findByText(/安装状态已变化/)).toBeInTheDocument();
    expect(
      await screen.findByText("已是最新", { selector: ".headline" }),
    ).toBeInTheDocument();
  });

  it("warns when MSIX is planned but App Installer is missing, and flips to portable", async () => {
    const user = userEvent.setup();
    api.winPlanUpdate.mockResolvedValue(
      report({ capabilities: { ...CAPS_OK, appInstaller: unknown } }),
    );
    api.setSettings.mockImplementation((next: AppSettings) => Promise.resolve(next));
    renderWinHome();

    const switchBtn = await screen.findByRole("button", { name: /改用免安装版/ });
    await user.click(switchBtn);
    await waitFor(() =>
      expect(api.setSettings).toHaveBeenCalledWith(
        expect.objectContaining({ windowsInstallMode: "portable" }),
      ),
    );
    // Switching re-plans so the route (and the banner) can settle.
    await waitFor(() => expect(api.winPlanUpdate.mock.calls.length).toBeGreaterThanOrEqual(2));
  });

  it("closes the install-dir sheet when a focus re-check finds the install drifted", async () => {
    const user = userEvent.setup();
    // Capture the focus-recheck listener so the test can fire it.
    let onFocus: (() => void) | undefined;
    vi.mocked(listen).mockImplementation((event: string, cb: unknown) => {
      if (event === "tauri://focus") onFocus = cb as () => void;
      return Promise.resolve(() => {});
    });
    // Portable fresh-install: no install yet, so clicking install opens the
    // install-dir sheet instead of running straight away.
    api.getSettings.mockResolvedValue(settings({ windowsInstallMode: "portable" }));
    api.winStatus.mockResolvedValue({ installed: null, status: "none" });
    api.winPlanUpdate.mockResolvedValue(
      report({ installed: null, plan: { ...PLAN_UPDATE, route: "portable-fallback" } }),
    );
    renderWinHome();

    await user.click(await screen.findByRole("button", { name: /安装 Codex/ }));
    // The location sheet is up (fresh portable install needs a target).
    expect(await screen.findByText("选择安装位置")).toBeInTheDocument();

    // Codex appears out-of-band; the focus probe now sees a managed install —
    // an identity change from the null snapshot the sheet was built against.
    api.winStatus.mockResolvedValue(STATUS_MANAGED);
    await waitFor(() => expect(onFocus).toBeDefined());
    await act(async () => {
      onFocus?.();
      await Promise.resolve();
    });

    // The stale install-dir sheet must be gone — otherwise its 使用当前位置 /
    // 浏览 buttons could still run install against the vanished snapshot,
    // bypassing the external→adopt boundary.
    await waitFor(() => expect(screen.queryByText("选择安装位置")).not.toBeInTheDocument());
  });

  it.each([
    { intent: "pause" as const, outcome: "false" as const },
    { intent: "pause" as const, outcome: "reject" as const },
    { intent: "cancel" as const, outcome: "false" as const },
    { intent: "cancel" as const, outcome: "reject" as const },
  ])(
    "keeps the Windows progress flow recoverable when $intent returns $outcome",
    async ({ intent, outcome }) => {
      const user = userEvent.setup();
      api.getSettings.mockResolvedValue(settings({ askBefore: false }));
      api.winPerformUpdate.mockImplementationOnce(() => new Promise<WinPerformReport>(() => {}));

      let onProgress: ((event: { payload: DownloadProgress }) => void) | undefined;
      listenMock.mockImplementation((event: string, cb: unknown) => {
        if (event === "win://download-progress") onProgress = cb as typeof onProgress;
        return Promise.resolve(() => {});
      });

      const stop = intent === "pause" ? api.winPauseDownload : api.winCancelDownload;
      if (outcome === "false") {
        stop.mockResolvedValue(false);
      } else {
        stop.mockRejectedValue(new Error("invoke bridge unavailable"));
      }

      renderWinHome();
      await user.click(await screen.findByRole("button", { name: /立即更新/ }));
      if (intent === "pause") {
        await waitFor(() => expect(onProgress).toBeDefined());
        act(() =>
          onProgress?.({
            payload: { downloaded: 10, total: 100, source: "mirror.example" },
          }),
        );
      }

      const action = intent === "pause" ? "暂停" : "取消";
      const button = await screen.findByRole("button", { name: action });
      await waitFor(() => expect(button).toBeEnabled());
      api.getOperationSnapshot.mockResolvedValue(ACTIVE_OPERATION);
      await user.click(button);

      const expected =
        outcome === "false"
          ? `${action}请求被后端拒绝。任务仍在继续，可重试。`
          : `${action}请求未送达。任务仍在继续，可重试。`;
      expect(await screen.findByRole("alert")).toHaveTextContent(expected);
      expect(screen.getByText("正在更新…")).toBeInTheDocument();
      expect(screen.getByRole("button", { name: action })).toBeEnabled();

      await user.click(screen.getByRole("button", { name: action }));
      await waitFor(() => expect(stop).toHaveBeenCalledTimes(2));
    },
  );

  it("keeps the Windows paused screen and both recovery actions when discard rejects", async () => {
    const user = userEvent.setup();
    api.getSettings.mockResolvedValue(settings({ askBefore: false }));
    api.winDiscardDownload.mockRejectedValueOnce(new Error("cache locked"));

    let rejectPerform: ((cause: unknown) => void) | undefined;
    let onProgress: ((event: { payload: DownloadProgress }) => void) | undefined;
    listenMock.mockImplementation((event: string, cb: unknown) => {
      if (event === "win://download-progress") onProgress = cb as typeof onProgress;
      return Promise.resolve(() => {});
    });
    api.winPerformUpdate.mockImplementationOnce(
      () => new Promise<WinPerformReport>((_resolve, reject) => (rejectPerform = reject)),
    );

    renderWinHome();
    await user.click(await screen.findByRole("button", { name: /立即更新/ }));
    await waitFor(() => expect(onProgress).toBeDefined());
    act(() =>
      onProgress?.({
        payload: { downloaded: 10, total: 100, source: "s", operationId: "op-active" },
      }),
    );
    await user.click(await screen.findByRole("button", { name: /^暂停$/ }));
    act(() => rejectPerform?.(new Error("download cancelled")));
    await screen.findByText("下载已暂停");

    await user.click(screen.getByRole("button", { name: "取消" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "取消未完成。下载仍处于暂停状态；你可以继续下载或重试取消。",
    );
    expect(screen.getByText("下载已暂停")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "继续" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "取消" })).toBeEnabled();

    await user.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() => expect(api.winDiscardDownload).toHaveBeenCalledTimes(2));
    expect(await screen.findByText("下载已取消。")).toBeInTheDocument();
  });
});
