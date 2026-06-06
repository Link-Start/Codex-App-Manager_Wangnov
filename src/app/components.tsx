import type { ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { Icon, type IconName } from "./icons";
import { useI18n } from "./i18n";

function closeWindow() {
  try {
    void getCurrentWindow().close();
  } catch {
    /* non-Tauri preview: no window to close */
  }
}

/** This is a normal window with no system title bar, so it draws its own close
 *  control. Closing quits the app (see the CloseRequested handler in lib.rs) —
 *  the app is meant to be opened when needed, not left resident. */
function CloseButton() {
  const { t } = useI18n();
  return (
    <button className="winclose" title={t("nav.close")} onClick={closeWindow}>
      <Icon name="close" />
    </button>
  );
}

export function TopBar({ children }: { children?: ReactNode }) {
  const { t } = useI18n();
  return (
    <div className="topbar">
      <div className="mark">C</div>
      <div className="wordmark">{t("app.name")}</div>
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
    <div className="navbar">
      <button className="navback" onClick={onBack} disabled={disableBack}>
        <Icon name="back" />
        {t("nav.back")}
      </button>
      <div className="navtitle">{title}</div>
      <div className="spacer" style={{ flex: 1 }} data-tauri-drag-region />
      {children}
      <CloseButton />
    </div>
  );
}

export type RingVariant = "accent" | "amber" | "muted" | "danger";

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
