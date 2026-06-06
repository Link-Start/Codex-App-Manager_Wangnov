import { useCallback, useEffect, useState } from "react";

import { managerApi } from "../services/managerApi";
import type {
  CapabilityCheck,
  InstallClass,
  MacInstallStatus,
  MacPerformReport,
  MacStageReport,
  MacUpdateReport,
  WinAutoStageReport,
  WinInstallStatus,
  WinPerformReport,
  WinStageReport,
  WinUninstallReport,
  WinUpdateReport,
} from "../shared/types";

type PlatformMode = "windows" | "macos";
type BusyState =
  | "win-plan"
  | "win-stage"
  | "win-perform"
  | "win-uninstall"
  | "mac-plan"
  | "mac-stage"
  | "mac-perform"
  | null;

const AUTO_DOWNLOAD_KEY = "codex-manager.win.autoDownload";
const AUTO_ALLOW_METERED_KEY = "codex-manager.win.autoAllowMetered";

function readStoredBool(key: string, fallback: boolean): boolean {
  if (typeof window === "undefined") {
    return fallback;
  }
  const value = window.localStorage.getItem(key);
  if (value === null) {
    return fallback;
  }
  return value === "true";
}

function writeStoredBool(key: string, value: boolean): void {
  if (typeof window !== "undefined") {
    window.localStorage.setItem(key, String(value));
  }
}

function mib(bytes: number | null | undefined): string {
  if (!bytes) {
    return "0.0";
  }
  return (bytes / 1_048_576).toFixed(1);
}

function statusLabel(status: InstallClass): string {
  return status === "managed" ? "manager 托管" : status === "external" ? "外部安装" : "未安装";
}

function checkLabel(check: CapabilityCheck): string {
  const prefix =
    check.state === "available" ? "可用" : check.state === "unavailable" ? "不可用" : "未知";
  return `${prefix} · ${check.detail}`;
}

export function App() {
  const [mode, setMode] = useState<PlatformMode>("windows");
  const [sim, setSim] = useState("");

  const [winReport, setWinReport] = useState<WinUpdateReport | null>(null);
  const [winStage, setWinStage] = useState<WinStageReport | null>(null);
  const [winAutoStage, setWinAutoStage] = useState<WinAutoStageReport | null>(null);
  const [winPerform, setWinPerform] = useState<WinPerformReport | null>(null);
  const [winUninstall, setWinUninstall] = useState<WinUninstallReport | null>(null);
  const [winStatus, setWinStatus] = useState<WinInstallStatus | null>(null);

  const [macReport, setMacReport] = useState<MacUpdateReport | null>(null);
  const [macStage, setMacStage] = useState<MacStageReport | null>(null);
  const [macPerform, setMacPerform] = useState<MacPerformReport | null>(null);
  const [macStatus, setMacStatus] = useState<MacInstallStatus | null>(null);

  const [busy, setBusy] = useState<BusyState>(null);
  const [winAutoBusy, setWinAutoBusy] = useState(false);
  const [autoDownloadEnabled, setAutoDownloadEnabled] = useState(() =>
    readStoredBool(AUTO_DOWNLOAD_KEY, true)
  );
  const [autoAllowMetered, setAutoAllowMetered] = useState(() =>
    readStoredBool(AUTO_ALLOW_METERED_KEY, false)
  );
  const [lastAutoStageKey, setLastAutoStageKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [mgrMsg, setMgrMsg] = useState<string | null>(null);
  const [mgrBusy, setMgrBusy] = useState(false);

  const simBuild = sim.trim() === "" ? undefined : Number(sim.trim());

  const winCheck = useCallback(async () => {
    setBusy("win-plan");
    setError(null);
    setWinStage(null);
    setWinAutoStage(null);
    setWinPerform(null);
    setWinUninstall(null);
    setLastAutoStageKey(null);
    try {
      setWinReport(await managerApi.winPlanUpdate());
      setWinStatus(await managerApi.winStatus());
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  }, []);

  const winDoStage = useCallback(async () => {
    setBusy("win-stage");
    setError(null);
    try {
      setWinStage(await managerApi.winStageUpdate());
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  }, []);

  const winCancelDownload = useCallback(async () => {
    setError(null);
    try {
      const cancelled = await managerApi.winCancelDownload();
      setWinAutoStage((current) => ({
        enabled: autoDownloadEnabled,
        allowMetered: autoAllowMetered,
        attempted: current?.attempted ?? true,
        skipped: true,
        reason: cancelled ? "cancel-requested" : "no-active-download",
        stage: current?.stage ?? null,
        capabilities: current?.capabilities ?? null,
        notes: [
          cancelled
            ? "已请求暂停下载；已下载的 .part 文件会保留供下次续传。"
            : "当前没有正在运行的下载。",
        ],
      }));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }, [autoAllowMetered, autoDownloadEnabled]);

  const changeAutoDownload = useCallback(
    (enabled: boolean) => {
      setAutoDownloadEnabled(enabled);
      if (!enabled && winAutoBusy) {
        void managerApi.winCancelDownload();
      }
    },
    [winAutoBusy]
  );

  const winDoPerform = useCallback(async () => {
    const confirmed = window.confirm(
      "将安装或更新 Windows Codex。后端会重新下载/校验 staging，并只尝试免提权 MSIX 侧载；失败不会改系统策略。"
    );
    if (!confirmed) {
      return;
    }
    setBusy("win-perform");
    setError(null);
    try {
      const result = await managerApi.winPerformUpdate(true);
      setWinPerform(result);
      setWinStage(result.stage);
      setWinStatus(await managerApi.winStatus());
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  }, []);

  const winDoUninstall = useCallback(async () => {
    const confirmed = window.confirm(
      "将卸载 manager 托管的 Windows Codex。默认保留 %USERPROFILE%\\.codex 用户数据。"
    );
    if (!confirmed) {
      return;
    }
    setBusy("win-uninstall");
    setError(null);
    try {
      const result = await managerApi.winUninstall(true, false);
      setWinUninstall(result);
      setWinStatus(await managerApi.winStatus());
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  }, []);

  const winAdopt = useCallback(async () => {
    setError(null);
    try {
      setWinStatus(await managerApi.winAdopt());
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }, []);

  const macCheck = useCallback(async () => {
    setBusy("mac-plan");
    setError(null);
    setMacStage(null);
    setMacPerform(null);
    try {
      setMacReport(await managerApi.macPlanUpdate(simBuild));
      setMacStatus(await managerApi.macStatus());
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  }, [simBuild]);

  const macDoStage = useCallback(async () => {
    setBusy("mac-stage");
    setError(null);
    setMacPerform(null);
    try {
      setMacStage(await managerApi.macStageUpdate(simBuild));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  }, [simBuild]);

  const macDoPerform = useCallback(async () => {
    const installed = macStatus?.installed;
    if (!installed || !macStage) {
      return;
    }
    const target = `build ${macStage.latestBuild}（${macStage.strategy}）`;
    // Name the ACTUAL detected install path (could be ~/Applications) so the
    // confirmation matches what the backend will replace.
    const path = installed.path;
    const ok = window.confirm(
      `即将用 ${target} 替换 ${path}：\n\n` +
        "· 会请求 Codex 优雅退出（绝不强杀，正在进行的任务可先保存）\n" +
        "· 替换前会复验 Apple 代码签名（发布者 = OpenAI）\n" +
        "· 失败会自动回滚到当前版本\n\n确定现在执行吗？",
    );
    if (!ok) {
      return;
    }
    setBusy("mac-perform");
    setError(null);
    try {
      const result = await managerApi.macPerformUpdate({
        fromBuild: installed.build,
        toBuild: macStage.latestBuild,
        path: installed.path,
      });
      setMacPerform(result);
      // The install is now manager-managed and on a new build — refresh status.
      await managerApi.macStatus().then(setMacStatus).catch(() => undefined);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  }, [macStage, macStatus]);

  const macAdopt = useCallback(async () => {
    setError(null);
    try {
      setMacStatus(await managerApi.macAdopt());
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }, []);

  const checkManager = useCallback(async () => {
    setMgrBusy(true);
    setMgrMsg(null);
    try {
      setMgrMsg(await managerApi.checkManagerUpdate());
    } catch (cause) {
      setMgrMsg("检查失败：" + (cause instanceof Error ? cause.message : String(cause)));
    } finally {
      setMgrBusy(false);
    }
  }, []);

  const winPlan = winReport?.plan ?? null;
  const macPlan = macReport?.plan ?? null;

  useEffect(() => {
    writeStoredBool(AUTO_DOWNLOAD_KEY, autoDownloadEnabled);
  }, [autoDownloadEnabled]);

  useEffect(() => {
    writeStoredBool(AUTO_ALLOW_METERED_KEY, autoAllowMetered);
  }, [autoAllowMetered]);

  useEffect(() => {
    if (mode === "windows") {
      void winCheck();
    } else {
      void macCheck();
    }
  }, [macCheck, mode, winCheck]);

  useEffect(() => {
    if (mode !== "windows" || !winPlan || winPlan.upToDate || !autoDownloadEnabled) {
      return;
    }
    if (winAutoBusy || winStage?.installReady) {
      return;
    }
    const autoKey = `${winPlan.packageMoniker}:${autoDownloadEnabled}:${autoAllowMetered}`;
    if (lastAutoStageKey === autoKey) {
      return;
    }
    setLastAutoStageKey(autoKey);
    setWinAutoBusy(true);
    void managerApi
      .winAutoStageUpdate(autoDownloadEnabled, autoAllowMetered)
      .then((result) => {
        setWinAutoStage(result);
        if (result.stage) {
          setWinStage(result.stage);
        }
      })
      .catch((cause) => {
        setWinAutoStage({
          enabled: autoDownloadEnabled,
          allowMetered: autoAllowMetered,
          attempted: true,
          skipped: false,
          reason: "error",
          stage: null,
          capabilities: null,
          notes: [cause instanceof Error ? cause.message : String(cause)],
        });
      })
      .finally(() => {
        setWinAutoBusy(false);
      });
  }, [
    autoAllowMetered,
    autoDownloadEnabled,
    lastAutoStageKey,
    mode,
    winAutoBusy,
    winPlan,
    winStage?.installReady,
  ]);

  const winCanStage =
    Boolean(winPlan) && !winPlan?.upToDate && busy === null && !winAutoBusy && !mgrBusy;
  const macCanStage = Boolean(macPlan) && !macPlan?.upToDate && busy === null && !mgrBusy;
  // Only after a real, EdDSA-verified stage (no sim-build preview) may the
  // destructive swap run — the delta basis must be the true installed bundle.
  // This is an UPDATE flow, so it also requires a detected install: without one
  // the backend rejects immediately ("no Codex detected to update").
  const macCanPerform =
    Boolean(macStage?.verified) &&
    !macStage?.upToDate &&
    Boolean(macStatus?.installed) &&
    simBuild === undefined &&
    busy === null &&
    !mgrBusy;

  return (
    <main className="wrap">
      <header className="head">
        <div className="logo">C</div>
        <div>
          <h1>
            Codex 更新器{" "}
            <span className="tag">
              {mode === "windows" ? "Windows · MSIX α" : "macOS · Sparkle delta"}
            </span>
          </h1>
          <p className="sub">以 mirror 为源 · 原生签名为信任锚 · 非破坏性 staging</p>
        </div>
      </header>

      <section className="card platform-card">
        <div className="switch">
          <button className={mode === "windows" ? "active" : ""} onClick={() => setMode("windows")}>
            Windows
          </button>
          <button className={mode === "macos" ? "active" : ""} onClick={() => setMode("macos")}>
            macOS
          </button>
        </div>
      </section>

      {error ? <div className="err">{error}</div> : null}

      {mode === "windows" ? (
        <>
          <section className="card">
            <div className="row">
              <div className="field">
                <label>已装 Codex</label>
                <div className="val">
                  <span>
                    {winStatus?.installed ? `version ${winStatus.installed.version}` : "未检测到"}
                    {winStatus ? (
                      <span className={`statuspill ${winStatus.status}`}>
                        {statusLabel(winStatus.status)}
                      </span>
                    ) : null}
                  </span>
                  {winStatus?.installed ? (
                    <span className="path">
                      {winStatus.installed.source} · {winStatus.installed.path}
                    </span>
                  ) : null}
                </div>
              </div>
              <div className="field">
                <label>建议路径</label>
                <div className="val">
                  <span>{winReport?.capabilities.recommendation ?? "..."}</span>
                  <span className="path">{winPlan?.packageMoniker ?? "..."}</span>
                </div>
              </div>
            </div>

            {winStatus?.status === "external" ? (
              <div className="adopt">
                <span>发现外部安装。纳入 manager 管理后由其负责后续更新。</span>
                <button className="btn" onClick={winAdopt} disabled={busy !== null}>
                  纳管
                </button>
              </div>
            ) : null}

            <div className="actions">
              <button className="btn" onClick={winCheck} disabled={busy !== null}>
                {busy === "win-plan" ? "检查中..." : "检查 Windows 更新"}
              </button>
              <button className="btn primary" onClick={winDoStage} disabled={!winCanStage}>
                {busy === "win-stage" ? "下载验签中..." : "下载并验签 MSIX"}
              </button>
              <button className="btn primary" onClick={winDoPerform} disabled={!winCanStage}>
                {busy === "win-perform" ? "执行中..." : "安装/更新"}
              </button>
              <button
                className="btn danger"
                onClick={winDoUninstall}
                disabled={busy !== null || winStatus?.status !== "managed"}
              >
                {busy === "win-uninstall" ? "卸载中..." : "卸载"}
              </button>
            </div>
          </section>

          <section className="card">
            <h2>Windows 后台预下载</h2>
            <div className="toggle-grid">
              <label className="toggle-row">
                <input
                  type="checkbox"
                  checked={autoDownloadEnabled}
                  onChange={(event) => changeAutoDownload(event.target.checked)}
                />
                <span>自动预下载</span>
              </label>
              <label className="toggle-row">
                <input
                  type="checkbox"
                  checked={autoAllowMetered}
                  onChange={(event) => {
                    setAutoAllowMetered(event.target.checked);
                    setLastAutoStageKey(null);
                  }}
                  disabled={!autoDownloadEnabled}
                />
                <span>允许计量网络</span>
              </label>
            </div>
            <div className="actions compact">
              {winAutoBusy ? (
                <button className="btn" onClick={winCancelDownload}>
                  暂停
                </button>
              ) : null}
              <span className={`inline-status ${winAutoBusy ? "active" : ""}`}>
                {winAutoBusy
                  ? "预下载中"
                  : winStage?.installReady
                    ? "已就绪"
                    : winAutoStage?.skipped
                      ? "已跳过"
                      : "待命"}
              </span>
            </div>
            {winAutoStage ? (
              <p className="note">
                {winAutoStage.reason}
                {winAutoStage.notes.length ? ` · ${winAutoStage.notes.join(" ")}` : ""}
              </p>
            ) : null}
          </section>

          {winReport ? (
            <section className="card">
              <h2>Windows 能力体检</h2>
              <ul className="kv">
                <li>
                  <span>Add-AppxPackage</span>
                  <b>{checkLabel(winReport.capabilities.addAppxPackage)}</b>
                </li>
                <li>
                  <span>AppXSvc</span>
                  <b>{checkLabel(winReport.capabilities.appxService)}</b>
                </li>
                <li>
                  <span>侧载策略</span>
                  <b>{checkLabel(winReport.capabilities.sideloadPolicy)}</b>
                </li>
                <li>
                  <span>计量网络</span>
                  <b>{checkLabel(winReport.capabilities.meteredNetwork)}</b>
                </li>
              </ul>
            </section>
          ) : null}

          {winPlan ? (
            <section className="card">
              <h2>Windows 更新计划</h2>
              {winPlan.upToDate ? (
                <p className="ok">已是最新 version {winPlan.latestVersion}</p>
              ) : (
                <div className="plan">
                  <div className={`pill ${winPlan.route === "msix-sideload" ? "delta" : "full"}`}>
                    {winPlan.route === "msix-sideload" ? "MSIX 侧载优先" : "便携回退"}
                  </div>
                  <ul className="kv">
                    <li>
                      <span>目标</span>
                      <b>{winPlan.latestVersion}</b>
                    </li>
                    <li>
                      <span>当前</span>
                      <b>{winPlan.currentVersion ?? "未安装"}</b>
                    </li>
                    <li>
                      <span>下载</span>
                      <b>{mib(winPlan.downloadSize)} MB</b>
                    </li>
                    <li>
                      <span>SHA256</span>
                      <b className="path">{winPlan.sha256}</b>
                    </li>
                    <li>
                      <span>便携回退</span>
                      <b>{winPlan.portableFallbackReady ? "可用" : "不可用"}</b>
                    </li>
                  </ul>
                  {winPlan.warnings.length ? (
                    <p className="note">{winPlan.warnings.join(" ")}</p>
                  ) : null}
                </div>
              )}
            </section>
          ) : null}

          {winStage ? (
            <section className="card">
              <h2>Windows 暂存结果</h2>
              {winStage.upToDate ? (
                <p className="ok">已是最新</p>
              ) : (
                <ul className="kv">
                  <li>
                    <span>SHA256</span>
                    <b className={winStage.hashVerified ? "save" : "bad"}>
                      {winStage.hashVerified ? "通过" : "未通过"}
                    </b>
                  </li>
                  <li>
                    <span>Authenticode</span>
                    <b className={winStage.authenticode?.trusted ? "save" : "bad"}>
                      {winStage.authenticode?.status ?? "未验证"}
                    </b>
                  </li>
                  <li>
                    <span>发布者</span>
                    <b className="path">{winStage.authenticode?.subject ?? "..."}</b>
                  </li>
                  <li>
                    <span>MSIX identity</span>
                    <b>
                      {winStage.identity?.name ?? "..."} ·{" "}
                      {winStage.identity?.processorArchitecture ?? "..."}
                    </b>
                  </li>
                  <li>
                    <span>暂存于</span>
                    <b className="path">{winStage.stagedPath}</b>
                  </li>
                </ul>
              )}
              {winStage.notes.length ? <p className="note">{winStage.notes.join(" ")}</p> : null}
            </section>
          ) : null}

          {winPerform ? (
            <section className="card">
              <h2>Windows 执行结果</h2>
              <ul className="kv">
                <li>
                  <span>结果</span>
                  <b className={winPerform.success ? "save" : "bad"}>
                    {winPerform.success ? "成功" : "未完成"}
                  </b>
                </li>
                <li>
                  <span>动作</span>
                  <b>{winPerform.action}</b>
                </li>
                <li>
                  <span>消息</span>
                  <b>{winPerform.message}</b>
                </li>
                <li>
                  <span>安装位置</span>
                  <b className="path">{winPerform.installed?.path ?? "未检测到"}</b>
                </li>
                <li>
                  <span>版本</span>
                  <b>{winPerform.installed?.version ?? "..."}</b>
                </li>
              </ul>
              {winPerform.notes.length ? <p className="note">{winPerform.notes.join(" ")}</p> : null}
            </section>
          ) : null}

          {winUninstall ? (
            <section className="card">
              <h2>Windows 卸载结果</h2>
              <ul className="kv">
                <li>
                  <span>结果</span>
                  <b className={winUninstall.success ? "save" : "bad"}>
                    {winUninstall.success ? "成功" : "未完成"}
                  </b>
                </li>
                <li>
                  <span>动作</span>
                  <b>{winUninstall.action}</b>
                </li>
                <li>
                  <span>消息</span>
                  <b>{winUninstall.message}</b>
                </li>
                <li>
                  <span>用户数据</span>
                  <b>{winUninstall.purgedUserData ? "已清除" : "已保留"}</b>
                </li>
              </ul>
              {winUninstall.notes.length ? <p className="note">{winUninstall.notes.join(" ")}</p> : null}
            </section>
          ) : null}
        </>
      ) : (
        <>
          <section className="card">
            <div className="row">
              <div className="field">
                <label>已装 Codex</label>
                <div className="val">
                  <span>
                    {macStatus?.installed ? `build ${macStatus.installed.build}` : "未检测到"}
                    {macStatus ? (
                      <span className={`statuspill ${macStatus.status}`}>
                        {statusLabel(macStatus.status)}
                      </span>
                    ) : null}
                  </span>
                  {macStatus?.installed ? <span className="path">{macStatus.installed.path}</span> : null}
                </div>
              </div>
              <div className="field">
                <label>模拟已装 build</label>
                <input
                  value={sim}
                  onChange={(event) => {
                    setSim(event.target.value);
                    // A prior stage was computed for a different basis, so it
                    // must never enable the destructive apply button.
                    setMacStage(null);
                    setMacPerform(null);
                  }}
                  placeholder="例如 3511"
                  inputMode="numeric"
                />
              </div>
            </div>

            {macStatus?.status === "external" ? (
              <div className="adopt">
                <span>发现外部安装的 Codex。纳入 manager 管理后由其负责后续更新。</span>
                <button className="btn" onClick={macAdopt} disabled={busy !== null || mgrBusy}>
                  纳管
                </button>
              </div>
            ) : null}

            <div className="actions">
              <button className="btn" onClick={macCheck} disabled={busy !== null || mgrBusy}>
                {busy === "mac-plan" ? "检查中..." : "检查 macOS 更新"}
              </button>
              <button className="btn primary" onClick={macDoStage} disabled={!macCanStage}>
                {busy === "mac-stage" ? "下载验签中..." : "下载并验签"}
              </button>
              {macStage?.verified && !macStage.upToDate && macStatus?.installed ? (
                <button className="btn danger" onClick={macDoPerform} disabled={!macCanPerform}>
                  {busy === "mac-perform" ? "替换中..." : "应用更新(替换并重启)"}
                </button>
              ) : null}
            </div>
            {macStage?.verified && !macStage.upToDate && simBuild !== undefined ? (
              <p className="note">
                当前为「模拟已装 build」预览态：真实替换已禁用。清空模拟值并重新「下载并验签」后方可应用。
              </p>
            ) : null}
          </section>

          {macPlan ? (
            <section className="card">
              <h2>macOS 更新计划</h2>
              {macPlan.upToDate ? (
                <p className="ok">已是最新 build {macPlan.latestBuild}</p>
              ) : (
                <div className="plan">
                  <div className={`pill ${macPlan.strategy.kind}`}>
                    {macPlan.strategy.kind === "delta" ? "增量 DELTA" : "全量 FULL"}
                  </div>
                  <ul className="kv">
                    <li>
                      <span>目标</span>
                      <b>
                        {macPlan.latestShortVersion} (build {macPlan.latestBuild})
                      </b>
                    </li>
                    <li>
                      <span>当前</span>
                      <b>build {macPlan.currentBuild}</b>
                    </li>
                    <li>
                      <span>下载</span>
                      <b>{mib(macPlan.downloadSize)} MB</b>
                    </li>
                    <li>
                      <span>全量</span>
                      <b>{mib(macPlan.fullSize)} MB</b>
                    </li>
                    <li>
                      <span>节省</span>
                      <b className="save">{macPlan.savingsPct.toFixed(1)}%</b>
                    </li>
                  </ul>
                  <div className="bar" title="下载量占全量比例">
                    <div
                      className="bar-fill"
                      style={{ width: `${Math.max(2, 100 - macPlan.savingsPct)}%` }}
                    />
                  </div>
                </div>
              )}
            </section>
          ) : null}

          {macStage ? (
            <section className="card">
              <h2>macOS 暂存结果</h2>
              {macStage.upToDate ? (
                <p className="ok">已是最新</p>
              ) : (
                <ul className="kv">
                  <li>
                    <span>策略</span>
                    <b>{macStage.strategy}</b>
                  </li>
                  <li>
                    <span>下载</span>
                    <b>
                      {mib(macStage.downloadSize)} MB（省 {macStage.savingsPct.toFixed(1)}%）
                    </b>
                  </li>
                  <li>
                    <span>EdDSA 验签</span>
                    <b className={macStage.verified ? "save" : "bad"}>
                      {macStage.verified ? "通过" : "未通过"}
                    </b>
                  </li>
                  <li>
                    <span>暂存于</span>
                    <b className="path">{macStage.stagedPath}</b>
                  </li>
                </ul>
              )}
              <p className="note">
                非破坏性：仅下载 + 验签到 staging，未触碰安装根。点「应用更新」才会 BinaryDelta apply → codesign 复验 → 原子替换 → 重启。
              </p>
            </section>
          ) : null}

          {macPerform ? (
            <section className="card">
              <h2>macOS 应用结果</h2>
              {macPerform.upToDate ? (
                <p className="ok">{macPerform.message}</p>
              ) : (
                <ul className="kv">
                  <li>
                    <span>结果</span>
                    <b className={macPerform.rolledBack ? "bad" : "save"}>
                      {macPerform.rolledBack ? "已回滚" : "已更新"}
                    </b>
                  </li>
                  <li>
                    <span>版本</span>
                    <b>
                      build {macPerform.fromBuild} → {macPerform.toBuild}（
                      {macPerform.strategy}）
                    </b>
                  </li>
                  <li>
                    <span>签名闸</span>
                    <b className={macPerform.verified ? "save" : "bad"}>
                      {macPerform.verified ? "EdDSA + codesign(OpenAI)" : "未通过"}
                    </b>
                  </li>
                  <li>
                    <span>重启</span>
                    <b>{macPerform.relaunched ? "已重启 Codex" : "未重启(原本未运行)"}</b>
                  </li>
                </ul>
              )}
              <p className="note">{macPerform.message}</p>
            </section>
          ) : null}
        </>
      )}

      <section className="card">
        <h2>manager 自更新</h2>
        <div className="actions">
          <button className="btn" onClick={checkManager} disabled={mgrBusy || busy !== null}>
            {mgrBusy ? "检查中..." : busy !== null ? "更新进行中..." : "检查 manager 更新"}
          </button>
        </div>
        {mgrMsg ? <p className="note">{mgrMsg}</p> : null}
      </section>

      <footer className="foot">
        {mode === "windows"
          ? `manifest: ${winReport?.manifestUrl ?? "..."}`
          : `appcast: ${macReport?.appcastUrl ?? "..."}`}
      </footer>
    </main>
  );
}
