import { Activity, Download, RefreshCw, ShieldCheck, Trash2 } from "lucide-react";

import { EndpointList } from "../components/EndpointList";
import { HealthList } from "../components/HealthList";
import { PlanView } from "../components/PlanView";
import { StatusBadge } from "../components/StatusBadge";
import { useManager } from "../hooks/useManager";

export function App() {
  const manager = useManager();
  const snapshot = manager.snapshot;

  return (
    <main className="shell">
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark">C</div>
          <div>
            <h1>Codex App Manager</h1>
            <p>{snapshot?.target.label ?? "Loading target"}</p>
          </div>
        </div>

        <nav className="nav">
          <a className="nav-item active" href="#overview">
            <Activity size={18} />
            <span>Overview</span>
          </a>
          <a className="nav-item" href="#payload">
            <Download size={18} />
            <span>Payload</span>
          </a>
          <a className="nav-item" href="#security">
            <ShieldCheck size={18} />
            <span>Verification</span>
          </a>
        </nav>
      </aside>

      <section className="workspace">
        <header className="topbar">
          <div>
            <p className="eyebrow">Mirror operations</p>
            <h2>Install, update, and remove managed Codex payloads</h2>
          </div>
          <div className="topbar-actions">
            <button
              className="icon-button"
              title="Check mirror update"
              onClick={manager.checkUpdates}
              disabled={manager.busy}
            >
              <RefreshCw size={18} />
            </button>
            <button className="primary-button" onClick={manager.planInstall} disabled={manager.busy}>
              <Download size={18} />
              <span>Plan install</span>
            </button>
            <button className="danger-button" onClick={manager.planUninstall} disabled={manager.busy}>
              <Trash2 size={18} />
              <span>Plan uninstall</span>
            </button>
          </div>
        </header>

        {manager.error ? <div className="error-strip">{manager.error}</div> : null}

        <section id="overview" className="overview-grid">
          <div className="summary-panel">
            <div className="panel-heading">
              <span>Manager</span>
              <StatusBadge tone="success" label={snapshot ? `v${snapshot.managerVersion}` : "loading"} />
            </div>
            <dl className="kv-list">
              <div>
                <dt>Platform</dt>
                <dd>{snapshot?.target.label ?? "..."}</dd>
              </div>
              <div>
                <dt>Install root</dt>
                <dd>{snapshot?.settings.installRoot ?? "..."}</dd>
              </div>
              <div>
                <dt>Payload status</dt>
                <dd>{snapshot?.installation.status ?? "..."}</dd>
              </div>
            </dl>
          </div>

          <div className="summary-panel">
            <div className="panel-heading">
              <span>Policy</span>
              <StatusBadge tone="neutral" label="managed" />
            </div>
            <div className="policy-stack">
              <div>
                <strong>Official payloads</strong>
                <span>MSIX and DMG remain external release assets.</span>
              </div>
              <div>
                <strong>User data</strong>
                <span>Preserved unless a purge action is selected.</span>
              </div>
              <div>
                <strong>Rollback</strong>
                <span>Planned through staging and previous-version retention.</span>
              </div>
            </div>
          </div>
        </section>

        <section id="payload" className="content-grid">
          <PlanView plan={manager.plan} updateCheck={manager.updateCheck} />
          <EndpointList endpoints={snapshot?.endpoints} />
        </section>

        <section id="security" className="content-grid">
          <HealthList report={manager.health} onRefresh={manager.refreshHealth} busy={manager.busy} />
          <div className="notes-panel">
            <h3>Verification queue</h3>
            <ul>
              <li>Compare mirrored asset SHA256 before touching install roots.</li>
              <li>Read package metadata before choosing an installer strategy.</li>
              <li>Keep Codex user data outside managed app removal.</li>
              <li>Record provenance for every install and update.</li>
            </ul>
          </div>
        </section>
      </section>
    </main>
  );
}

