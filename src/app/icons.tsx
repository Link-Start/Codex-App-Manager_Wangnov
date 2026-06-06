// Stroke icon set (currentColor, 24×24). One consistent weight, friendly caps.

import type { ReactNode } from "react";

export type IconName =
  | "check"
  | "alert"
  | "arrowUp"
  | "download"
  | "loader"
  | "refresh"
  | "gear"
  | "shield"
  | "info"
  | "chevron"
  | "back"
  | "trash"
  | "sliders"
  | "message"
  | "globe"
  | "external"
  | "close";

const PATHS: Record<IconName, ReactNode> = {
  check: <polyline points="4 12.5 9.5 18 20 6.5" />,
  alert: (
    <>
      <path d="M12 4 2.6 20h18.8L12 4Z" />
      <line x1="12" y1="10" x2="12" y2="14" />
      <circle cx="12" cy="17.4" r="0.5" />
    </>
  ),
  arrowUp: (
    <>
      <line x1="12" y1="19" x2="12" y2="5.5" />
      <polyline points="6 11 12 5 18 11" />
    </>
  ),
  download: (
    <>
      <path d="M12 3.5v11" />
      <polyline points="7.5 10 12 14.5 16.5 10" />
      <path d="M5 19.5h14" />
    </>
  ),
  loader: <path d="M12 3.5a8.5 8.5 0 1 0 8.5 8.5" />,
  refresh: (
    <>
      <path d="M20 11.5a8 8 0 1 0-1.9 6.4" />
      <polyline points="20 4.5 20 11.5 13 11.5" />
    </>
  ),
  gear: (
    <>
      <circle cx="12" cy="12" r="3" />
      <path d="M12 2.5v2.4M12 19.1v2.4M21.5 12h-2.4M4.9 12H2.5M18.7 5.3l-1.7 1.7M7 17l-1.7 1.7M18.7 18.7 17 17M7 7 5.3 5.3" />
    </>
  ),
  shield: (
    <>
      <path d="M12 3 5 5.7v5.3c0 4.3 3 7.6 7 9.5 4-1.9 7-5.2 7-9.5V5.7L12 3Z" />
      <polyline points="9 12 11 14 15 9.6" />
    </>
  ),
  info: (
    <>
      <circle cx="12" cy="12" r="8.5" />
      <line x1="12" y1="11" x2="12" y2="16.5" />
      <circle cx="12" cy="7.6" r="0.5" />
    </>
  ),
  chevron: <polyline points="9 6 15 12 9 18" />,
  back: <polyline points="15 6 9 12 15 18" />,
  trash: (
    <>
      <path d="M5 7h14" />
      <path d="M9.5 7V5.2A1.2 1.2 0 0 1 10.7 4h2.6a1.2 1.2 0 0 1 1.2 1.2V7" />
      <path d="M6.5 7l.8 12.1A1.3 1.3 0 0 0 8.6 20.4h6.8a1.3 1.3 0 0 0 1.3-1.3L17.5 7" />
    </>
  ),
  sliders: (
    <>
      <line x1="4" y1="8" x2="20" y2="8" />
      <line x1="4" y1="16" x2="20" y2="16" />
      <circle cx="9" cy="8" r="2.3" />
      <circle cx="15" cy="16" r="2.3" />
    </>
  ),
  message: (
    <path d="M5 5h14a1.5 1.5 0 0 1 1.5 1.5v8A1.5 1.5 0 0 1 19 16H9l-4 3.5V6.5A1.5 1.5 0 0 1 5 5Z" />
  ),
  globe: (
    <>
      <circle cx="12" cy="12" r="8.5" />
      <path d="M3.5 12h17M12 3.5c2.5 2.4 2.5 14.6 0 17M12 3.5c-2.5 2.4-2.5 14.6 0 17" />
    </>
  ),
  external: (
    <>
      <path d="M14 5h5v5" />
      <path d="M19 5l-8 8" />
      <path d="M18 14v4.5A1.5 1.5 0 0 1 16.5 20h-11A1.5 1.5 0 0 1 4 18.5v-11A1.5 1.5 0 0 1 5.5 6H10" />
    </>
  ),
  close: (
    <>
      <line x1="6" y1="6" x2="18" y2="18" />
      <line x1="18" y1="6" x2="6" y2="18" />
    </>
  ),
};

export function Icon({ name, className }: { name: IconName; className?: string }) {
  return (
    <svg
      className={className}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {PATHS[name]}
    </svg>
  );
}
