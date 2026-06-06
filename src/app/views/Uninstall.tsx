import { useState } from "react";

import { managerApi } from "../../services/managerApi";
import { Icon } from "../icons";
import { useI18n } from "../i18n";
import { NavBar, Ring, Toggle } from "../components";

export function Uninstall({ onBack }: { onBack: () => void }) {
  const { t } = useI18n();
  // Default to keeping the user's data (~/.codex). Opting out is deliberate.
  const [keepData, setKeepData] = useState(true);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await managerApi.macUninstall(keepData);
      setDone(r.keptCodexHome ? t("uninstall.doneKept") : t("uninstall.doneCleared"));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="pop">
      <NavBar title={t("uninstall.heading")} onBack={onBack} />
      <div className="scroll view">
        {done ? (
          <>
            <section className="hero" style={{ marginTop: 16 }}>
              <Ring icon="check" />
              <div className="headline">{t("uninstall.heading")}</div>
              <div className="desc">{done}</div>
            </section>
            <button className="btn ghost big" onClick={onBack}>
              {t("nav.back")}
            </button>
          </>
        ) : (
          <>
            <section className="hero" style={{ marginTop: 8 }}>
              <Ring icon="trash" variant="danger" />
              <div className="headline" style={{ fontSize: 18 }}>
                {t("uninstall.heading")}
              </div>
              <div className="desc">{t("uninstall.warn")}</div>
            </section>

            <div className="list">
              <div className="row">
                <span className="rtext">
                  <span className="rtitle">{t("uninstall.keepData")}</span>
                  <span className="rsub">{t("uninstall.keepDataNote")}</span>
                </span>
                <Toggle checked={keepData} onChange={setKeepData} disabled={busy} />
              </div>
            </div>

            {error ? (
              <div className="banner err">
                <Icon name="alert" />
                <span>{error}</span>
              </div>
            ) : null}

            <div className="actions">
              <button className="btn danger big" onClick={run} disabled={busy}>
                <Icon name="trash" />
                {busy ? t("uninstall.working") : t("uninstall.confirm")}
              </button>
              <button className="btn ghost" onClick={onBack} disabled={busy}>
                {t("uninstall.cancel")}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
