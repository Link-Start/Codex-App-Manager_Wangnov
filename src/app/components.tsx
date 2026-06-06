import { useEffect, useState, type ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { managerApi } from "../services/managerApi";
import { Icon, type IconName, CodexMark } from "./icons";
import { useI18n } from "./i18n";

function closeWindow() {
  try {
    void getCurrentWindow().close();
  } catch {
    /* non-Tauri preview: no window to close */
  }
}

/** This is a normal window with no system title bar, so it draws its own close
 *  control. Closing quits the app (CloseRequested → exit in lib.rs). By default
 *  it asks first (so you don't lose an in-progress op by mis-clicking); the
 *  "关闭前确认" setting turns that off. */
function CloseButton() {
  const { t } = useI18n();
  const [confirm, setConfirm] = useState(false);
  const [ask, setAsk] = useState(true);

  useEffect(() => {
    void managerApi
      .getSettings()
      .then((s) => setAsk(s.confirmClose))
      .catch(() => undefined);
  }, []);

  return (
    <>
      <button
        className="winclose"
        title={t("nav.close")}
        onClick={() => (ask ? setConfirm(true) : closeWindow())}
      >
        <Icon name="close" />
      </button>
      {confirm ? (
        <div className="scrim" onClick={() => setConfirm(false)}>
          <div className="sheet" onClick={(e) => e.stopPropagation()}>
            <Ring icon="info" variant="amber" />
            <h3>{t("close.confirm.title")}</h3>
            <p>{t("close.confirm.body")}</p>
            <div className="row2">
              <button className="btn ghost" onClick={() => setConfirm(false)}>
                {t("confirm.cancel")}
              </button>
              <button className="btn primary" onClick={closeWindow}>
                {t("close.confirm.ok")}
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </>
  );
}

// The window is frameless, so the bar drags it. tauri's data-tauri-drag-region
// triggers on the element actually clicked, so every non-button element in the
// bar carries it; the buttons deliberately don't (they stay clickable).
export function TopBar({ children }: { children?: ReactNode }) {
  const { t } = useI18n();
  return (
    <div className="topbar" data-tauri-drag-region>
      <div className="mark" data-tauri-drag-region>
        <CodexMark />
      </div>
      <div className="wordmark" data-tauri-drag-region>
        {t("app.name")}
      </div>
      <div className="spacer" data-tauri-drag-region />
      {children}
      <CloseButton />
    </div>
  );
}

export function NavBar({
  title,
  onBack,
  disableBack = false,
  children,
}: {
  title: string;
  onBack: () => void;
  disableBack?: boolean;
  children?: ReactNode;
}) {
  const { t } = useI18n();
  return (
    <div className="navbar" data-tauri-drag-region>
      <button className="navback" onClick={onBack} disabled={disableBack}>
        <Icon name="back" />
        {t("nav.back")}
      </button>
      <div className="navtitle" data-tauri-drag-region>
        {title}
      </div>
      <div className="spacer" style={{ flex: 1 }} data-tauri-drag-region />
      {children}
      <CloseButton />
    </div>
  );
}

export type RingVariant = "accent" | "success" | "amber" | "muted" | "danger";

export function Ring({
  icon,
  variant = "accent",
  spin = false,
  className = "",
}: {
  icon: IconName;
  variant?: RingVariant;
  spin?: boolean;
  className?: string;
}) {
  const v = variant === "accent" ? "" : variant;
  return (
    <div className={`ring ${v} ${spin ? "spin" : ""} ${className}`}>
      <Icon name={icon} />
    </div>
  );
}

export function Toggle({
  checked,
  onChange,
  disabled = false,
}: {
  checked: boolean;
  onChange?: (v: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <button
      className="toggle"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange?.(!checked)}
    />
  );
}
