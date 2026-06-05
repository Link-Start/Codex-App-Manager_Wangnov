import { useCallback, useEffect, useState } from "react";

import { managerApi } from "../services/managerApi";
import type {
  HealthReport,
  ManagerSnapshot,
  OperationPlan,
  PayloadUpdateCheck,
} from "../shared/types";

export function useManager() {
  const [snapshot, setSnapshot] = useState<ManagerSnapshot | null>(null);
  const [plan, setPlan] = useState<OperationPlan | null>(null);
  const [updateCheck, setUpdateCheck] = useState<PayloadUpdateCheck | null>(null);
  const [health, setHealth] = useState<HealthReport | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const run = useCallback(async <T,>(task: () => Promise<T>, onSuccess: (value: T) => void) => {
    setBusy(true);
    setError(null);
    try {
      onSuccess(await task());
    } catch (cause) {
      const message = cause instanceof Error ? cause.message : String(cause);
      setError(message);
    } finally {
      setBusy(false);
    }
  }, []);

  const refreshSnapshot = useCallback(() => {
    return run(managerApi.getSnapshot, setSnapshot);
  }, [run]);

  const refreshHealth = useCallback(() => {
    return run(managerApi.runHealthCheck, setHealth);
  }, [run]);

  const planInstall = useCallback(() => {
    return run(managerApi.planInstall, setPlan);
  }, [run]);

  const planUninstall = useCallback(() => {
    return run(managerApi.planUninstall, setPlan);
  }, [run]);

  const checkUpdates = useCallback(() => {
    return run(managerApi.checkUpdates, setUpdateCheck);
  }, [run]);

  useEffect(() => {
    void refreshSnapshot();
    void refreshHealth();
    void planInstall();
  }, [planInstall, refreshHealth, refreshSnapshot]);

  return {
    busy,
    checkUpdates,
    error,
    health,
    plan,
    planInstall,
    planUninstall,
    refreshHealth,
    refreshSnapshot,
    snapshot,
    updateCheck,
  };
}

