import { RotateCw } from "lucide-react";

import type { HealthReport } from "../shared/types";

interface HealthListProps {
  busy: boolean;
  onRefresh: () => void;
  report: HealthReport | null;
}

export function HealthList({ busy, onRefresh, report }: HealthListProps) {
  return (
    <div className="health-panel">
      <div className="panel-title-row">
        <h3>Health</h3>
        <button className="secondary-button" onClick={onRefresh} disabled={busy}>
          <RotateCw size={16} />
          <span>Refresh</span>
        </button>
      </div>
      <ul className="health-list">
        {report?.checks.map((check) => (
          <li className="health-item" key={check.id}>
            <span className={`health-dot ${check.status}`} />
            <span>
              <strong>{check.label}</strong>
              <span>{check.detail}</span>
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

