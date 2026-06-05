import type { MirrorEndpoints } from "../shared/types";

interface EndpointListProps {
  endpoints: MirrorEndpoints | undefined;
}

export function EndpointList({ endpoints }: EndpointListProps) {
  return (
    <div className="endpoint-panel">
      <div className="panel-title-row">
        <h3>Mirror endpoints</h3>
      </div>
      <dl className="endpoint-list">
        {endpoints
          ? Object.entries(endpoints).map(([key, value]) => (
              <div key={key}>
                <dt>{key}</dt>
                <dd>{value}</dd>
              </div>
            ))
          : null}
      </dl>
    </div>
  );
}

