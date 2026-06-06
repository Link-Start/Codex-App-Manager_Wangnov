import type { ReactNode } from "react";

import { Icon, type IconName } from "./icons";
import { useI18n } from "./i18n";

export function TopBar({ children }: { children?: ReactNode }) {
  const { t } = useI18n();
  return (
    <div className="topbar">
      <div className="mark">C</div>
      <div className="wordmark">{t("app.name")}</div>
      <div className="spacer" />
      {children}
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
      <div className="spacer" style={{ flex: 1 }} />
      {children}
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
