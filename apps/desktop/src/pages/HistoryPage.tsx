import { useEffect, useState } from "react";
import { api, TransferRecord } from "../api";

const STATUS_ICON: Record<string, string> = {
  completed: "✅",
  failed:    "❌",
  in_progress: "⏳",
};

function formatBytes(bytes: number) {
  if (bytes > 1_000_000_000) return `${(bytes / 1e9).toFixed(1)} GB`;
  if (bytes > 1_000_000)     return `${(bytes / 1e6).toFixed(1)} MB`;
  return `${(bytes / 1000).toFixed(0)} KB`;
}

function groupByDate(records: TransferRecord[]): Record<string, TransferRecord[]> {
  const groups: Record<string, TransferRecord[]> = {};
  const now = Date.now() / 1000;

  for (const r of records) {
    const diff = now - r.created_at;
    let label = "Earlier";
    if (diff < 86400)  label = "Today";
    else if (diff < 172800) label = "Yesterday";
    (groups[label] ??= []).push(r);
  }
  return groups;
}

export default function HistoryPage() {
  const [records, setRecords] = useState<TransferRecord[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    api.listTransfers()
      .then(setRecords)
      .finally(() => setLoading(false));
  }, []);

  const groups = groupByDate(records);

  return (
    <div>
      <h1 className="mb-6 text-xl font-semibold">Transfer History</h1>

      {loading ? (
        <p className="text-slate-400">Loading…</p>
      ) : records.length === 0 ? (
        <div className="flex flex-col items-center gap-3 pt-20 text-slate-400">
          <span className="text-5xl">📋</span>
          <p>No transfers yet</p>
        </div>
      ) : (
        Object.entries(groups).map(([label, items]) => (
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
                  <span className="text-xl">{STATUS_ICON[r.status]}</span>
                  <div className="flex-1 min-w-0">
                    <p className="truncate font-medium text-slate-100">{r.file_name}</p>
                    <p className="text-sm text-slate-400">
                      {r.direction === "sent" ? "↑ To" : "↓ From"} {r.device_name} ·{" "}
                      {formatBytes(r.file_size)}
                    </p>
                  </div>
                  {r.status === "failed" && (
                    <button className="shrink-0 rounded-lg bg-tovi-blue px-3 py-1 text-xs text-white hover:bg-blue-500">
                      Retry
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
