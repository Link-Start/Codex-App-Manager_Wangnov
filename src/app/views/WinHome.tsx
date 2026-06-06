import { useCallback, useEffect, useMemo, useState } from "react";

import { managerApi } from "../../services/managerApi";
import type {
  AppSettings,
  WinInstallStatus,
  WinPerformReport,
  WinUpdateReport,
} from "../../shared/types";
import { DEFAULT_SETTINGS } from "../../shared/types";
import { Icon } from "../icons";
import { useI18n, type TKey } from "../i18n";
import { Ring, TopBar } from "../components";

function mib(bytes: number): string {
  return `${(bytes / 1_048_576).toFixed(1)} MB`;
}

type Kind = "loading" | "error" | "none" | "idle" | "update" | "external" | "uptodate";

// Windows counterpart of MacHome — same design system + state machine, driven by
// the win_* backend (codex-win-engine): MSIX sideload or portable fallback.
export function WinHome({ onOpenSettings }: { onOpenSettings: () => void }) {
  const { t } = useI18n();
  const [report, setReport] = useState<WinUpdateReport | null>(null);
  const [status, setStatus] = useState<WinInstallStatus | null>(null);
  const [perform, setPerform] = useState<WinPerformReport | null>(null);
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
  const [busy, setBusy] = useState<"plan" | "perform" | "adopt" | "install" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [statusLoaded, setStatusLoaded] = useState(false);
  const [statusFailed, setStatusFailed] = useState(false);

  const check = useCallback(async () => {
    setBusy("plan");
    setError(null);
    try {
      setReport(await managerApi.winPlanUpdate());
    } catch (cause) {
      setReport(null);
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  }, []);

  const refreshStatus = useCallback(async () => {
    try {
      setStatus(await managerApi.winStatus());
      setStatusFailed(false);
    } catch {
      setStatusFailed(true);
    } finally {
      setStatusLoaded(true);
    }
  }, []);

  useEffect(() => {
    void (async () => {
      const s = await managerApi.getSettings().catch(() => DEFAULT_SETTINGS);
      setSettings(s);
      void refreshStatus();
      if (s.autoCheck) {
        void check();
      }
    })();
  }, [check, refreshStatus]);

  const adopt = useCallback(async () => {
    setBusy("adopt");
    setError(null);
    try {
      setStatus(await managerApi.winAdopt());
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  }, []);

  // Windows install + update both go through win_perform_update (the route —
  // MSIX sideload or portable fallback — is decided by the backend plan).
  const runPerform = useCallback(
    async (mode: "perform" | "install") => {
      setBusy(mode);
      setError(null);
      try {
        const result = await managerApi.winPerformUpdate(true);
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
    },
    [refreshStatus, check],
  );

  const plan = report?.plan ?? null;
  const installed = status?.installed ?? report?.installed ?? null;
  const isManaged = status?.status === "managed";
  const updateAvailable = Boolean(plan) && !plan?.upToDate;
  const routeNote =
    plan?.route === "portable-fallback" ? t("win.route.portable") : t("win.route.msix");

  const kind: Kind = useMemo(() => {
    if (!installed) {
      if (busy === "plan" || !statusLoaded) return "loading";
      if (statusFailed || error) return "error";
      return "none";
    }
    if (!statusLoaded) return "loading";
    if (status?.status === "external") return "external";
    if (busy === "plan" && !report) return "loading";
    if (error && !report) return "error";
    if (!report) return "idle";
    if (updateAvailable) return "update";
    return "uptodate";
  }, [busy, report, error, installed, updateAvailable, status, statusLoaded, statusFailed]);

  const version = installed?.version || plan?.latestVersion || "";
  const sourceLabel = t(`source.${settings.source}` as TKey);

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
        <button className="iconbtn" title={t("home.recheck")} onClick={check} disabled={busy !== null}>
          <Icon name="refresh" />
        </button>
        <button className="iconbtn" title={t("nav.settings")} onClick={onOpenSettings}>
          <Icon name="gear" />
        </button>
      </TopBar>

      <div className="scroll view">
        {perform ? (
          <div className={`banner ${perform.success ? "ok" : "err"}`}>
            <Icon name={perform.success ? "check" : "alert"} />
            <span>{perform.message}</span>
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
              <div className="sub">{t("win.installSub", { version })}</div>
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
                <span className="ver">{plan?.latestVersion}</span>
              </div>
              <div className="flow">
                {t("home.update.flow", {
                  from: plan?.currentVersion ?? version,
                  to: plan?.latestVersion ?? "",
                })}
                {plan?.downloadSize
                  ? ` · ${t("home.update.size", { size: mib(plan.downloadSize) })}`
                  : ""}
              </div>
              <div className="microcue">
                <Icon name="shield" />
                {routeNote}
              </div>
            </>
          ) : kind === "external" ? (
            <>
              <Ring icon="shield" variant="amber" />
              <div className="headline">{t("home.external.title")}</div>
              <div className="sub">{t("win.installSub", { version })}</div>
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
              onClick={() => (settings.askBefore ? setConfirmOpen(true) : void runPerform("perform"))}
              disabled={busy !== null}
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
            <button
              className="btn primary big"
              onClick={() => runPerform("install")}
              disabled={busy !== null}
            >
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
            <Ring icon="arrowUp" />
            <h3>{t("confirm.title", { version: plan.latestVersion })}</h3>
            <p>
              {t("win.confirm.body")}
              <br />
              {routeNote}
            </p>
            <div className="row2">
              <button className="btn ghost" onClick={() => setConfirmOpen(false)}>
                {t("confirm.cancel")}
              </button>
              <button className="btn primary" onClick={() => runPerform("perform")}>
                {t("confirm.ok")}
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}
