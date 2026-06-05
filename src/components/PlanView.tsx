import type { OperationPlan, PayloadUpdateCheck } from "../shared/types";
import { StatusBadge } from "./StatusBadge";

interface PlanViewProps {
  plan: OperationPlan | null;
  updateCheck: PayloadUpdateCheck | null;
}

export function PlanView({ plan, updateCheck }: PlanViewProps) {
  const label = plan ? `${plan.kind} / ${plan.strategy}` : "waiting";

  return (
    <div className="plan-panel">
      <div className="panel-title-row">
        <h3>Operation plan</h3>
        <StatusBadge tone={plan ? "success" : "neutral"} label={label} />
      </div>

      {updateCheck ? (
        <div className="update-message">
          {updateCheck.status}: {updateCheck.message}
        </div>
      ) : null}

      {plan ? (
        <ol className="step-list">
          {plan.steps.map((step, index) => (
            <li className="step-item" key={step.id}>
              <span className="step-index">{index + 1}</span>
              <span className="step-copy">
                <strong>{step.title}</strong>
                <span>{step.detail}</span>
              </span>
            </li>
          ))}
        </ol>
      ) : (
        <div className="empty-state">No plan loaded.</div>
      )}
    </div>
  );
}

