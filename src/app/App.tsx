import { useCallback, useEffect, useState } from "react";

import { managerApi } from "../services/managerApi";
import type { MacStageReport, MacUpdateReport } from "../shared/types";

function mib(bytes: number): string {
  return (bytes / 1_048_576).toFixed(1);
}

export function App() {
  const [sim, setSim] = useState("");
  const [report, setReport] = useState<MacUpdateReport | null>(null);
  const [stage, setStage] = useState<MacStageReport | null>(null);
  const [busy, setBusy] = useState<"plan" | "stage" | null>(null);
  const [error, setError] = useState<string | null>(null);

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
    try {
      setStage(await managerApi.macStageUpdate(simBuild));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  }, [simBuild]);

  useEffect(() => {
    void check();
    // run once on mount
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const plan = report?.plan ?? null;
  const canStage = Boolean(plan) && !plan?.upToDate && busy === null;

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
              {report?.installed ? `build ${report.installed.build}` : "未检测到"}
              {report?.installed ? <span className="path">{report.installed.path}</span> : null}
            </div>
          </div>
          <div className="field">
            <label>模拟已装 build(演示用 · 留空=真实)</label>
            <input
              value={sim}
              onChange={(event) => setSim(event.target.value)}
              placeholder="例如 3511"
              inputMode="numeric"
            />
          </div>
        </div>
        <div className="actions">
          <button className="btn" onClick={check} disabled={busy !== null}>
            {busy === "plan" ? "检查中…" : "检查更新"}
          </button>
          <button className="btn primary" onClick={doStage} disabled={!canStage}>
            {busy === "stage" ? "下载验签中…" : "下载并验签(暂存)"}
          </button>
        </div>
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
            非破坏性：仅下载 + 验签到 staging，未触碰安装根。下一步才是 BinaryDelta apply → codesign 复验 → 原子替换。
          </p>
        </section>
      ) : null}

      <footer className="foot">appcast: {report?.appcastUrl ?? "…"} · 仅 macOS · v1 雏形</footer>
    </main>
  );
}
