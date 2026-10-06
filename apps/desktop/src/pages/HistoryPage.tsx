import { useCallback, useEffect, useState } from "react";
import { api, subscribe, TransferRecord } from "../api";
import { formatBytes } from "../format";

const STATUS_ICON: Record<TransferRecord["status"], string> = {
  completed: "✅",
  failed: "❌",
  in_progress: "⏳",
};

function groupByDate(records: TransferRecord[]): [string, TransferRecord[]][] {
  const groups = new Map<string, TransferRecord[]>();
  const now = Date.now() / 1000;
  for (const r of records) {
    const diff = now - r.created_at;
    const label = diff < 86400 ? "Today" : diff < 172800 ? "Yesterday" : "Earlier";
    groups.set(label, [...(groups.get(label) ?? []), r]);
  }
  return [...groups.entries()];
}

export default function HistoryPage() {
  const [records, setRecords] = useState<TransferRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [retrying, setRetrying] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setRecords(await api.listTransfers());
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
    const stops = [subscribe("transfer:finished", refresh), subscribe("transfer:started", refresh)];
    return () => stops.forEach((stop) => stop());
  }, [refresh]);

  const retry = async (r: TransferRecord) => {
    if (!r.path) return;
    setRetrying(r.id);
    try {
      await api.sendFile(r.device_id, r.path);
    } catch (e) {
      setError(String(e));
    } finally {
      setRetrying(null);
      refresh();
    }
  };

  return (
    <div>
      <h1 className="mb-6 text-xl font-semibold">Transfer History</h1>
      {error && <p className="mb-4 text-sm text-red-400">{error}</p>}

      {loading ? (
        <p className="text-slate-400">Loading…</p>
      ) : records.length === 0 ? (
        <div className="flex flex-col items-center gap-3 pt-20 text-slate-400">
          <span className="text-5xl">📋</span>
          <p>No transfers yet</p>
        </div>
      ) : (
        groupByDate(records).map(([label, items]) => (
          <section key={label} className="mb-6">
            <h2 className="mb-2 text-xs font-semibold uppercase tracking-wider text-slate-500">
              {label}
            </h2>
            <ul className="space-y-2">
              {items.map((r) => (
                <li
                  key={r.id}
                  className="flex items-center gap-4 rounded-xl border border-tovi-border bg-tovi-panel px-5 py-3"
                >
                  <span className="text-xl" title={r.status.replace("_", " ")}>
                    {STATUS_ICON[r.status]}
                  </span>
                  <div className="min-w-0 flex-1">
                    <p className="truncate font-medium text-slate-100">{r.file_name}</p>
                    <p className="text-sm text-slate-400">
                      {r.direction === "sent" ? "↑ To" : "↓ From"} {r.device_name} ·{" "}
                      {formatBytes(r.file_size)}
                      {r.status === "in_progress" ? " · unfinished" : ""}
                    </p>
                    {r.status === "failed" && r.error && (
                      <p className="truncate text-xs text-red-400" title={r.error}>
                        {r.error}
                      </p>
                    )}
                  </div>
                  {r.direction === "sent" && r.status !== "completed" && r.path && (
                    <button
                      onClick={() => retry(r)}
                      disabled={retrying !== null}
                      className="shrink-0 rounded-lg bg-tovi-blue px-3 py-1 text-xs text-white hover:bg-blue-500 disabled:opacity-50"
                    >
                      {retrying === r.id ? "Sending…" : r.status === "failed" ? "Retry" : "Resume"}
                    </button>
                  )}
                </li>
              ))}
            </ul>
          </section>
        ))
      )}
    </div>
  );
}
