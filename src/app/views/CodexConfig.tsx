import { useI18n } from "../i18n";
import { NavBar, Ring } from "../components";

// Reserved section for the upcoming ~/.codex management (sessions / auth /
// config). The route, nav entry and view shell exist so the feature drops in
// without restructuring; the backend port is stubbed in app/codex_config.
export function CodexConfig({ onBack }: { onBack: () => void }) {
  const { t } = useI18n();
  return (
    <div className="pop">
      <NavBar title={t("nav.config")} onBack={onBack} />
      <div className="scroll view">
        <section className="hero" style={{ marginTop: 24 }}>
          <Ring icon="sliders" variant="muted" />
          <div className="headline" style={{ fontSize: 18 }}>
            {t("nav.config")}
          </div>
          <div className="desc">{t("config.desc")}</div>
          <span className="tag soon" style={{ marginTop: 14 }}>
            {t("config.soon")}
          </span>
        </section>
      </div>
    </div>
  );
}
