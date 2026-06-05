import { useCallback, useEffect, useState } from "react";

import { managerApi } from "../services/managerApi";
import type {
  InstallClass,
  MacInstallStatus,
  MacPerformReport,
  MacStageReport,
  MacUpdateReport,
} from "../shared/types";

function mib(bytes: number): string {
  return (bytes / 1_048_576).toFixed(1);
}

function statusLabel(status: InstallClass): string {
  return status === "managed" ? "manager 托管" : status === "external" ? "外部安装" : "未安装";
}

export function App() {
  const [sim, setSim] = useState("");
  const [report, setReport] = useState<MacUpdateReport | null>(null);
  const [stage, setStage] = useState<MacStageReport | null>(null);
  const [perform, setPerform] = useState<MacPerformReport | null>(null);
  const [status, setStatus] = useState<MacInstallStatus | null>(null);
  const [busy, setBusy] = useState<"plan" | "stage" | "perform" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [mgrMsg, setMgrMsg] = useState<string | null>(null);
  const [mgrBusy, setMgrBusy] = useState(false);

  const simBuild = sim.trim() === "" ? undefined : Number(sim.trim());

  const check = useCallback(async () => {
    setBusy("plan");
    setError(null);
    setStage(null);
    try {
      setReport(await managerApi.macPlanUpdate(simBuild));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  }, [simBuild]);

  const doStage = useCallback(async () => {
    setBusy("stage");
    setError(null);
    setPerform(null);
    try {
      setStage(await managerApi.macStageUpdate(simBuild));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  }, [simBuild]);

  const doPerform = useCallback(async () => {
    const target = stage
      ? `build ${stage.latestBuild}（${stage.strategy}）`
      : "最新版本";
    // Name the ACTUAL detected install path (could be ~/Applications) so the
    // confirmation matches what the backend will replace.
    const path = status?.installed?.path ?? "/Applications/Codex.app";
    const ok = window.confirm(
      `即将用 ${target} 替换 ${path}：\n\n` +
        "· 会请求 Codex 优雅退出（绝不强杀，正在进行的任务可先保存）\n" +
        "· 替换前会复验 Apple 代码签名（发布者 = OpenAI）\n" +
        "· 失败会自动回滚到当前版本\n\n确定现在执行吗？",
    );
    if (!ok) {
      return;
    }
    setBusy("perform");
    setError(null);
    try {
      const result = await managerApi.macPerformUpdate(true);
      setPerform(result);
      // The install is now manager-managed and on a new build — refresh status.
      await managerApi.macStatus().then(setStatus).catch(() => undefined);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  }, [stage, status]);

  const adopt = useCallback(async () => {
    setError(null);
    try {
      setStatus(await managerApi.macAdopt());
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

  useEffect(() => {
    void check();
    void managerApi.macStatus().then(setStatus).catch(() => undefined);
    // run once on mount
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const plan = report?.plan ?? null;
  const canStage = Boolean(plan) && !plan?.upToDate && busy === null;
  // Only after a real, EdDSA-verified stage (no sim-build preview) may the
  // destructive swap run — the delta basis must be the true installed bundle.
  const canPerform =
    Boolean(stage?.verified) &&
    !stage?.upToDate &&
    simBuild === undefined &&
    busy === null;

  return (
    <main className="wrap">
      <header className="head">
        <div className="logo">C</div>
        <div>
          <h1>
            Codex 更新器 <span className="tag">macOS · 雏形</span>
          </h1>
          <p className="sub">以 mirror 为源 · 复用 Sparkle delta · 钉死 EdDSA 验签</p>
        </div>
      </header>

      <section className="card">
        <div className="row">
          <div className="field">
            <label>已装 Codex</label>
            <div className="val">
              <span>
                {status?.installed ? `build ${status.installed.build}` : "未检测到"}
                {status ? (
                  <span className={`statuspill ${status.status}`}>{statusLabel(status.status)}</span>
                ) : null}
              </span>
              {status?.installed ? <span className="path">{status.installed.path}</span> : null}
            </div>
          </div>
          <div className="field">
            <label>模拟已装 build(演示用 · 留空=真实)</label>
            <input
              value={sim}
              onChange={(event) => {
                setSim(event.target.value);
                // A prior stage was computed for a different basis — invalidate
                // it so it can never enable the destructive apply.
                setStage(null);
                setPerform(null);
              }}
              placeholder="例如 3511"
              inputMode="numeric"
            />
          </div>
        </div>

        {status?.status === "external" ? (
          <div className="adopt">
            <span>发现外部安装的 Codex（官方 / 商店）。纳入 manager 管理后由其负责后续更新。</span>
            <button className="btn" onClick={adopt} disabled={busy !== null}>
              纳管
            </button>
          </div>
        ) : null}

        <div className="actions">
          <button className="btn" onClick={check} disabled={busy !== null}>
            {busy === "plan" ? "检查中…" : "检查更新"}
          </button>
          <button className="btn primary" onClick={doStage} disabled={!canStage}>
            {busy === "stage" ? "下载验签中…" : "下载并验签(暂存)"}
          </button>
          {stage?.verified && !stage.upToDate ? (
            <button className="btn danger" onClick={doPerform} disabled={!canPerform}>
              {busy === "perform" ? "替换中…" : "应用更新(替换并重启)"}
            </button>
          ) : null}
        </div>
        {stage?.verified && !stage.upToDate && simBuild !== undefined ? (
          <p className="note">
            当前为「模拟已装 build」预览态：真实替换已禁用。清空模拟值并重新「下载并验签」后方可应用。
          </p>
        ) : null}
      </section>

      {error ? <div className="err">⚠ {error}</div> : null}

      {plan ? (
        <section className="card">
          <h2>更新计划</h2>
          {plan.upToDate ? (
            <p className="ok">✓ 已是最新(build {plan.latestBuild})</p>
          ) : (
            <div className="plan">
              <div className={`pill ${plan.strategy.kind}`}>
                {plan.strategy.kind === "delta" ? "增量 DELTA" : "全量 FULL"}
              </div>
              <ul className="kv">
                <li>
                  <span>目标</span>
                  <b>
                    {plan.latestShortVersion} (build {plan.latestBuild})
                  </b>
                </li>
                <li>
                  <span>当前</span>
                  <b>build {plan.currentBuild}</b>
                </li>
                <li>
                  <span>下载</span>
                  <b>{mib(plan.downloadSize)} MB</b>
                </li>
                <li>
                  <span>全量</span>
                  <b>{mib(plan.fullSize)} MB</b>
                </li>
                <li>
                  <span>节省</span>
                  <b className="save">{plan.savingsPct.toFixed(1)}%</b>
                </li>
              </ul>
              <div className="bar" title="下载量占全量比例">
                <div
                  className="bar-fill"
                  style={{ width: `${Math.max(2, 100 - plan.savingsPct)}%` }}
                />
              </div>
            </div>
          )}
        </section>
      ) : null}

      {stage ? (
        <section className="card">
          <h2>暂存结果</h2>
          {stage.upToDate ? (
            <p className="ok">✓ 已是最新</p>
          ) : (
            <ul className="kv">
              <li>
                <span>策略</span>
                <b>{stage.strategy}</b>
              </li>
              <li>
                <span>下载</span>
                <b>
                  {mib(stage.downloadSize)} MB（省 {stage.savingsPct.toFixed(1)}%）
                </b>
              </li>
              <li>
                <span>EdDSA 验签</span>
                <b className={stage.verified ? "save" : "bad"}>
                  {stage.verified ? "✅ 通过(钉死公钥)" : "❌ 未通过"}
                </b>
              </li>
              <li>
                <span>暂存于</span>
                <b className="path">{stage.stagedPath}</b>
              </li>
            </ul>
          )}
          <p className="note">
            非破坏性：仅下载 + 验签到 staging，未触碰安装根。点「应用更新」才会 BinaryDelta apply → codesign 复验 → 原子替换 → 重启。
          </p>
        </section>
      ) : null}

      {perform ? (
        <section className="card">
          <h2>应用结果</h2>
          {perform.upToDate ? (
            <p className="ok">✓ {perform.message}</p>
          ) : (
            <ul className="kv">
              <li>
                <span>结果</span>
                <b className={perform.rolledBack ? "bad" : "save"}>
                  {perform.rolledBack ? "↩ 已回滚" : "✅ 已更新"}
                </b>
              </li>
              <li>
                <span>版本</span>
                <b>
                  build {perform.fromBuild} → {perform.toBuild}（{perform.strategy}）
                </b>
              </li>
              <li>
                <span>签名闸</span>
                <b className={perform.verified ? "save" : "bad"}>
                  {perform.verified ? "✅ EdDSA + codesign(OpenAI)" : "❌ 未通过"}
                </b>
              </li>
              <li>
                <span>重启</span>
                <b>{perform.relaunched ? "已重启 Codex" : "未重启(原本未运行)"}</b>
              </li>
            </ul>
          )}
          <p className="note">{perform.message}</p>
        </section>
      ) : null}

      <section className="card">
        <h2>manager 自更新</h2>
        <div className="actions">
          <button className="btn" onClick={checkManager} disabled={mgrBusy}>
            {mgrBusy ? "检查中…" : "检查 manager 更新"}
          </button>
        </div>
        {mgrMsg ? <p className="note">{mgrMsg}</p> : null}
      </section>

      <footer className="foot">appcast: {report?.appcastUrl ?? "…"} · 仅 macOS · v1 雏形</footer>
    </main>
  );
}
