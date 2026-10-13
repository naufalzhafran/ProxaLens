// Side-by-side summary and a unified line diff of two flows.
import { useEffect, useMemo, useState } from "react";
import { api, errorMessage, type FlowDetail } from "../api";
import { diffLines, hasChanges, requestLines, responseLines, type DiffLine } from "../lib/diff";
import { useUi } from "../state/ui";
import { Modal, Tabs } from "./ui";

export function DiffView({ ids }: { ids: [number, number] }) {
  const close = useUi((s) => s.close);
  const [flows, setFlows] = useState<[FlowDetail, FlowDetail] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<"request" | "response">("response");

  useEffect(() => {
    Promise.all(ids.map((id) => api.getFlow(id)))
      .then(([a, b]) => (a && b ? setFlows([a, b]) : setError("One of the flows no longer exists.")))
      .catch((e) => setError(errorMessage(e)));
  }, [ids]);

  const diffs = useMemo(() => {
    if (!flows) return null;
    const [a, b] = flows;
    return {
      request: diffLines(requestLines(a), requestLines(b)),
      response: diffLines(responseLines(a), responseLines(b)),
    };
  }, [flows]);

  return (
    <Modal title={`Compare #${ids[0]} with #${ids[1]}`} onClose={close} wide>
      {error && <p className="text-st-server">{error}</p>}
      {flows && diffs && (
        <>
          <div className="mb-3 grid grid-cols-2 gap-3 font-mono">
            {flows.map((f) => (
              <div key={f.meta.id} className="truncate rounded-md bg-surface px-2 py-1">
                <span className="text-muted">#{f.meta.id}</span> {f.meta.method} {f.meta.status ?? "—"} {f.meta.url}
              </div>
            ))}
          </div>
          <Tabs
            value={tab}
            onChange={setTab}
            tabs={[
              { id: "request", label: hasChanges(diffs.request) ? "Request •" : "Request" },
              { id: "response", label: hasChanges(diffs.response) ? "Response •" : "Response" },
            ]}
          />
          <Lines lines={diffs[tab]} />
        </>
      )}
    </Modal>
  );
}

function Lines({ lines }: { lines: DiffLine[] }) {
  if (!hasChanges(lines)) return <p className="p-3 text-muted">No differences.</p>;
  return (
    <pre className="selectable mt-2 font-mono text-xs">
      {lines.map((l, i) => (
        <div
          key={i}
          className={`whitespace-pre-wrap break-all px-2 ${
            l.type === "add"
              ? "bg-st-ok/10 text-st-ok"
              : l.type === "del"
                ? "bg-st-server/10 text-st-server"
                : ""
          }`}
        >
          <span className="mr-2 select-none text-muted">{l.type === "add" ? "+" : l.type === "del" ? "−" : " "}</span>
          {l.text || " "}
        </div>
      ))}
    </pre>
  );
}
