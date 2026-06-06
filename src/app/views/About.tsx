import { useCallback, useState } from "react";

import { managerApi } from "../../services/managerApi";
import { Icon } from "../icons";
import { useI18n } from "../i18n";
import { NavBar } from "../components";

const APP_VERSION = "0.1.0";
const REPO_URL = "https://github.com/Wangnov/Codex-App-Manager";

export function About({ onBack }: { onBack: () => void }) {
  const { t } = useI18n();
  const [mgrBusy, setMgrBusy] = useState(false);
  const [mgrMsg, setMgrMsg] = useState<string | null>(null);

  const checkManager = useCallback(async () => {
    setMgrBusy(true);
    setMgrMsg(null);
    try {
      setMgrMsg(await managerApi.checkManagerUpdate());
    } catch (cause) {
      setMgrMsg(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setMgrBusy(false);
    }
  }, []);

  return (
    <div className="pop">
      <NavBar title={t("settings.more.about")} onBack={onBack} />
      <div className="scroll view">
        <section className="hero" style={{ paddingTop: 8 }}>
          <div className="mark" style={{ width: 56, height: 56, borderRadius: 16, fontSize: 30, marginBottom: 14 }}>
            C
          </div>
          <div className="headline" style={{ fontSize: 18 }}>
            {t("app.name")}
          </div>
          <div className="sub">{t("about.version", { v: APP_VERSION })}</div>
          <div className="desc">{t("about.tagline")}</div>
        </section>

        <div className="list">
          <button className="row" onClick={checkManager} disabled={mgrBusy}>
            <Icon name="refresh" className="ricon" />
            <span className="rtext">
              <span className="rtitle">{t("about.checkManager")}</span>
              {mgrMsg ? <span className="rsub">{mgrMsg}</span> : null}
            </span>
            <span className="rval">{mgrBusy ? t("about.mgrChecking") : ""}</span>
          </button>
          <a className="row" href={REPO_URL} target="_blank" rel="noreferrer">
            <Icon name="message" className="ricon" />
            <span className="rtext">
              <span className="rtitle">{t("about.feedback")}</span>
              <span className="rsub">{REPO_URL.replace("https://", "")}</span>
            </span>
            <Icon name="external" className="chev" />
          </a>
        </div>
      </div>
    </div>
  );
}
