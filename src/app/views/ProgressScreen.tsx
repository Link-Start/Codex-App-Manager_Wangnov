import type { RefObject } from "react";

import type { DownloadProgress } from "../../shared/types";
import { Icon } from "../icons";
import { useI18n } from "../i18n";
import { Ring, TopBar } from "../components";
import { mib } from "../format";

export type DownloadStopIntent = "pause" | "cancel";

export interface PausedDownload {
  kind: "perform" | "install";
  dl: DownloadProgress | null;
}

/** The full-screen download/install progress view, shared by the Mac and
 *  Windows homes (byte-for-byte identical before this extraction). Owns the
 *  progressbar accessibility semantics so assistive tech can follow a
 *  destructive install/update: `role="progressbar"` + aria-value*, and an
 *  aria-live phase line ("preparing" → "downloading" → "finishing"). */
export function ProgressScreen({
  scene,
  scopeRef,
  paused,
  dl,
  dlPct,
  dlBytes,
  dlSpeed,
  installing,
  downloadStop,
  downloadStopBusy,
  onResume,
  onPause,
  onCancel,
}: {
  scene: string;
  scopeRef: RefObject<HTMLDivElement | null>;
  paused: PausedDownload | null;
  /** Live progress (null when not transferring). */
  dl: DownloadProgress | null;
  /** Eased display figures (useCountUp), so the readouts glide. */
  dlPct: number;
  dlBytes: number;
  dlSpeed: number;
  /** Whether the operation is a fresh install (vs an in-place update). */
  installing: boolean;
  downloadStop: DownloadStopIntent | null;
  downloadStopBusy: boolean;
  onResume: () => void;
  onPause: () => void;
  onCancel: () => void;
}) {
  const { t } = useI18n();

  // Paused reads from its captured snapshot; live runs from the eased `dl`.
  const snap = paused ? paused.dl : dl;
  const known = Boolean(snap && snap.total > 0);
  const snapPct = snap && snap.total > 0 ? Math.min(100, (snap.downloaded / snap.total) * 100) : 0;
  const pct = known ? Math.round(paused ? snapPct : dlPct) : null;
  const barPct = paused ? snapPct : dlPct;
  // Bytes are in → the uninterruptible install phase (gate/quit/atomic swap on
  // mac, sideload/extract on Windows). Say so and drop the dead buttons rather
  // than leave them greyed for no visible reason.
  const finishing = !paused && Boolean(snap && snap.total > 0 && snap.downloaded >= snap.total);
  // Pause only makes sense mid-transfer; cancel is the "abandon" out and works
  // through the preparing phase too (a backend abort checkpoint honors it), but
  // not once the install has begun.
  const canPause =
    !paused && Boolean(dl && dl.total > 0 && dl.downloaded < dl.total) && !downloadStopBusy;
  const canCancel = !paused && !finishing && !downloadStopBusy;

  const phase = paused
    ? t("progress.paused.title")
    : finishing
      ? t("progress.finishing")
      : snap
        ? t("progress.downloadingFrom", { source: snap.source })
        : t("progress.preparing");

  return (
    <div className="pop">
      <TopBar />
      <div className="scroll" ref={scopeRef}>
        <div className="hero" style={{ marginTop: 24 }} key={scene}>
          <Ring icon={paused ? "pause" : "loader"} spin={!paused} className="glow" />
          <div className={`headline${paused ? "" : " shimmer"}`}>
            {paused
              ? t("progress.paused.title")
              : installing
                ? t("progress.installing")
                : t("progress.title")}
          </div>
          {!paused ? (
            <div className="sub" aria-live="polite">
              {phase}
            </div>
          ) : null}
          {pct !== null ? (
            <div className="pctbig">
              {pct}
              <span className="pctsign">%</span>
            </div>
          ) : null}
          <div
            className="bar"
            role="progressbar"
            aria-label={installing ? t("progress.installing") : t("progress.title")}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={pct ?? undefined}
            aria-valuetext={pct !== null ? `${pct}% · ${phase}` : phase}
          >
            <div
              className={`bar-fill${pct === null ? " indeterminate" : ""}`}
              style={pct === null ? undefined : { width: `${barPct}%` }}
            />
          </div>
          {known && snap ? (
            <div className="dlmeta">
              {mib(paused ? snap.downloaded : dlBytes)} / {mib(snap.total)}
              {!paused && dlSpeed > 0 ? ` · ${mib(dlSpeed)}/s` : ""}
            </div>
          ) : null}
          <div className="progress-actions">
            {paused ? (
              <button className="btn primary" onClick={onResume}>
                <Icon name="play" />
                {t("progress.resume")}
              </button>
            ) : (
              <button className="btn ghost" onClick={onPause} disabled={!canPause}>
                <Icon name="pause" />
                {downloadStop === "pause" ? t("progress.pausePending") : t("progress.pause")}
              </button>
            )}
            <button
              className="btn danger"
              onClick={onCancel}
              disabled={!paused && !canCancel}
            >
              <Icon name="close" />
              {downloadStop === "cancel" ? t("progress.cancelPending") : t("progress.cancel")}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
