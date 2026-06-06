import { useCallback, useEffect, useMemo, useState } from "react";

import { managerApi } from "../../services/managerApi";
import type {
  AppSettings,
  MacInstallStatus,
  MacPerformReport,
  MacUpdateReport,
} from "../../shared/types";
import { DEFAULT_SETTINGS } from "../../shared/types";
import { Icon } from "../icons";
import { useI18n, type TKey } from "../i18n";
import { Ring, TopBar } from "../components";

function mib(bytes: number): string {
  return `${(bytes / 1_048_576).toFixed(1)} MB`;
}

type Kind = "loading" | "error" | "none" | "idle" | "update" | "external" | "uptodate";

export function Home({ onOpenSettings }: { onOpenSettings: () => void }) {
  const { t } = useI18n();
  const [report, setReport] = useState<MacUpdateReport | null>(null);
  const [status, setStatus] = useState<MacInstallStatus | null>(null);
  const [perform, setPerform] = useState<MacPerformReport | null>(null);
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
  const [busy, setBusy] = useState<"plan" | "perform" | "adopt" | "install" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);

  const check = useCallback(async () => {
    setBusy("plan");
    setError(null);
    try {
      setReport(await managerApi.macPlanUpdate());
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  }, []);

  const refreshStatus = useCallback(async () => {
    await managerApi.macStatus().then(setStatus).catch(() => undefined);
  }, []);

  useEffect(() => {
    void (async () => {
      const s = await managerApi.getSettings().catch(() => DEFAULT_SETTINGS);
      setSettings(s);
      void refreshStatus();
      // Honor "自动检查更新": only hit the appcast on open when enabled.
      if (s.autoCheck) {
        void check();
      }
    })();
  }, [check, refreshStatus]);

  const adopt = useCallback(async () => {
    setBusy("adopt");
    setError(null);
    try {
      setStatus(await managerApi.macAdopt());
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  }, []);

  const runInstall = useCallback(async () => {
    setBusy("install");
    setError(null);
    try {
      setStatus(await managerApi.macInstall());
      await check();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  }, [check]);

  const runPerform = useCallback(async () => {
    const installed = status?.installed;
    const plan = report?.plan;
    if (!installed || !plan || plan.upToDate) return;
    setBusy("perform");
    setError(null);
    try {
      const result = await managerApi.macPerformUpdate({
        fromBuild: installed.build,
        toBuild: plan.latestBuild,
        path: installed.path,
      });
      setPerform(result);
      setConfirmOpen(false);
      await refreshStatus();
      await check();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      setConfirmOpen(false);
    } finally {
      setBusy(null);
    }
  }, [status, report, refreshStatus, check]);

  const plan = report?.plan ?? null;
  const installed = status?.installed ?? report?.installed ?? null;
  const isManaged = status?.status === "managed";
  const updateAvailable = Boolean(plan) && !plan?.upToDate;

  const kind: Kind = useMemo(() => {
    if (busy === "plan" && !report) return "loading";
    if (error && !report) return "error";
    if (!installed) return "none";
    // Installed but not checked yet (auto-check disabled) — don't claim a state.
    if (!report) return "idle";
    if (updateAvailable) return "update";
    if (status?.status === "external") return "external";
    return "uptodate";
  }, [busy, report, error, installed, updateAvailable, status]);

  const version = plan?.latestShortVersion || (installed ? `build ${installed.build}` : "");
  const sourceLabel = t(`source.${settings.source}` as TKey);

  // ── progress (performing / installing) takes over the whole screen ─────────
  if (busy === "perform" || busy === "install") {
    return (
      <div className="pop">
        <TopBar />
        <div className="scroll view">
          <div className="hero" style={{ marginTop: 24 }}>
            <Ring icon="loader" spin />
            <div className="headline">
              {busy === "install" ? t("progress.installing") : t("progress.title")}
            </div>
            <div className="sub">{t("progress.downloading")}</div>
            <div className="bar">
              <div className="bar-fill" style={{ width: "62%" }} />
            </div>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="pop">
      <TopBar>
        <button
          className="iconbtn"
          title={t("home.recheck")}
          onClick={check}
          disabled={busy !== null}
        >
          <Icon name="refresh" />
        </button>
        <button className="iconbtn" title={t("nav.settings")} onClick={onOpenSettings}>
          <Icon name="gear" />
        </button>
      </TopBar>

      <div className="scroll view">
        {perform && !perform.rolledBack ? (
          <div className="banner ok">
            <Icon name="check" />
            <span>
              {t("success.title")} ·{" "}
              {perform.relaunched ? t("success.relaunched") : t("success.manualLaunch")}
            </span>
          </div>
        ) : null}
        {perform && perform.rolledBack ? (
          <div className="banner err">
            <Icon name="alert" />
            <span>{t("success.rolledBack")}</span>
          </div>
        ) : null}
        {error ? (
          <div className="banner err">
            <Icon name="alert" />
            <span>{error}</span>
          </div>
        ) : null}

        <section className="hero">
          {kind === "loading" ? (
            <>
              <Ring icon="loader" spin />
              <div className="headline">{t("home.checking")}</div>
            </>
          ) : kind === "error" ? (
            <>
              <Ring icon="alert" variant="danger" />
              <div className="headline">{t("home.error.title")}</div>
              <div className="desc">{t("home.error.sub")}</div>
            </>
          ) : kind === "none" ? (
            <>
              <Ring icon="download" variant="muted" />
              <div className="headline">{t("home.none.title")}</div>
              <div className="desc">{t("home.none.sub")}</div>
            </>
          ) : kind === "idle" ? (
            <>
              <Ring icon="shield" variant="muted" />
              <div className="headline">{t("home.idle.title")}</div>
              <div className="sub">
                {installed ? t("home.idle.sub", { build: installed.build }) : ""}
              </div>
              <div className="prov">
                <span className={`dot ${isManaged ? "managed" : "external"}`} />
                {isManaged ? t("prov.managed") : t("prov.external")}
              </div>
            </>
          ) : kind === "update" ? (
            <>
              <Ring icon="arrowUp" />
              <div className="headline">{t("home.update.title")}</div>
              <div className="sub">
                <span className="ver">{version}</span>
              </div>
              <div className="flow">
                {t("home.update.flow", {
                  from: `build ${plan?.currentBuild}`,
                  to: `build ${plan?.latestBuild}`,
                })}
                {plan ? ` · ${t("home.update.size", { size: mib(plan.downloadSize) })}` : ""}
              </div>
            </>
          ) : kind === "external" ? (
            <>
              <Ring icon="shield" variant="amber" />
              <div className="headline">{t("home.external.title")}</div>
              <div className="sub">
                <span className="ver">{version}</span>
              </div>
              <div className="prov">
                <span className="dot external" />
                {t("prov.external")}
              </div>
              <div className="desc">{t("home.external.desc")}</div>
            </>
          ) : (
            <>
              <Ring icon="check" />
              <div className="headline">{t("home.uptodate.title")}</div>
              <div className="sub">{t("home.uptodate.sub", { version })}</div>
              <div className="microcue">
                <Icon name="shield" />
                {t("home.official")} · {t("home.checkedJustNow")}
              </div>
            </>
          )}
        </section>

        <div className="actions">
          {kind === "update" ? (
            <button
              className="btn primary big"
              onClick={() => (settings.askBefore ? setConfirmOpen(true) : void runPerform())}
            >
              <Icon name="download" />
              {t("home.update.cta")}
            </button>
          ) : null}
          {kind === "idle" ? (
            <button className="btn primary big" onClick={check} disabled={busy !== null}>
              <Icon name="refresh" />
              {t("home.recheck")}
            </button>
          ) : null}
          {kind === "external" ? (
            <button className="btn primary big" onClick={adopt} disabled={busy !== null}>
              <Icon name="shield" />
              {t("home.external.cta")}
            </button>
          ) : null}
          {kind === "none" ? (
            <button className="btn primary big" onClick={runInstall} disabled={busy !== null}>
              <Icon name="download" />
              {t("home.none.cta")}
            </button>
          ) : null}
          {kind === "uptodate" ? (
            <button className="btn ghost big" onClick={check} disabled={busy !== null}>
              <Icon name="refresh" />
              {t("home.recheck")}
            </button>
          ) : null}
        </div>

        {installed ? (
          <div className="foot">
            {t("home.source", { source: sourceLabel })}
            <span>·</span>
            <button className="gobtn" onClick={onOpenSettings}>
              {t("nav.settings")}
            </button>
          </div>
        ) : null}
      </div>

      {confirmOpen && plan ? (
        <div className="scrim" onClick={() => setConfirmOpen(false)}>
          <div className="sheet" onClick={(e) => e.stopPropagation()}>
            <Ring icon="arrowUp" className="" />
            <h3>{t("confirm.title", { version })}</h3>
            <p>{t("confirm.body")}</p>
            <div className="row2">
              <button className="btn ghost" onClick={() => setConfirmOpen(false)}>
                {t("confirm.cancel")}
              </button>
              <button className="btn primary" onClick={runPerform}>
                {t("confirm.ok")}
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}
