import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";

import { withViewTransition } from "./viewTransition";

export type ThemeMode = "system" | "light" | "dark";
type Resolved = "light" | "dark";

interface ThemeCtx {
  mode: ThemeMode;
  resolved: Resolved;
  setMode: (m: ThemeMode) => void;
}

const Ctx = createContext<ThemeCtx | null>(null);
const LS_KEY = "cam.theme";

function readMode(): ThemeMode {
  const v = localStorage.getItem(LS_KEY);
  return v === "system" || v === "light" || v === "dark" ? v : "dark";
}

function systemPrefersDark(): boolean {
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? true;
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [mode, setModeState] = useState<ThemeMode>(readMode);
  const [systemDark, setSystemDark] = useState<boolean>(systemPrefersDark);

  // Track the OS appearance so "system" mode stays live.
  useEffect(() => {
    const mq = window.matchMedia?.("(prefers-color-scheme: dark)");
    if (!mq) return;
    const onChange = () => setSystemDark(mq.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  const resolved: Resolved = mode === "system" ? (systemDark ? "dark" : "light") : mode;

  useEffect(() => {
    document.documentElement.dataset.theme = resolved;
  }, [resolved]);

  const setMode = useCallback((m: ThemeMode) => {
    localStorage.setItem(LS_KEY, m);
    withViewTransition(() => {
      setModeState(m);
      // Set the attribute synchronously too, so the transition's "after"
      // snapshot already reflects the new theme — the effect below is passive
      // and would otherwise run after the snapshot is captured.
      document.documentElement.dataset.theme =
        m === "system" ? (systemPrefersDark() ? "dark" : "light") : m;
    });
  }, []);

  const value = useMemo(() => ({ mode, resolved, setMode }), [mode, resolved, setMode]);
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useTheme(): ThemeCtx {
  const ctx = useContext(Ctx);
  if (!ctx) throw new Error("useTheme must be used within ThemeProvider");
  return ctx;
}
