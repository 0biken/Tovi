import { useCallback, useEffect, useState } from "react";
import { ask } from "@tauri-apps/plugin-dialog";
import { api, subscribe, TransferRecord } from "../api";
import { formatBytes } from "../format";
import ActiveTransferList from "../components/ActiveTransferList";
import { useActiveTransferIds } from "../useActiveTransfers";

const STATUS_ICON: Record<TransferRecord["status"], string> = {
  completed: "✅",
  failed: "❌",
  in_progress: "⏳",
};

/** File types Windows runs when opened */
const RUNNABLE = /\.(exe|msi|bat|cmd|com|scr|ps1|vbs|vbe|js|jse|wsf|wsh|hta|lnk|reg|cpl|jar)$/i;

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
  const [query, setQuery] = useState("");
  const activeIds = useActiveTransferIds();

  const openFile = async (r: TransferRecord, reveal: boolean) => {
    try {
      if (reveal) return await api.revealTransferFile(r.id);
      // Opening a program runs it: check first when it came from another device
      if (
        r.direction === "received" &&
        RUNNABLE.test(r.file_name) &&
        !(await ask(`"${r.file_name}" is a program. Opening it will run it on this computer.`, {
          title: "Run this file?",
          kind: "warning",
          okLabel: "Run",
          cancelLabel: "Cancel",
        }))
      ) {
        return;
      }
      await api.openTransferFile(r.id);
    } catch (e) {
      setError(String(e));
    }
  };

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

  const q = query.trim().toLowerCase();
  const visible = records.filter(
    (r) =>
      !activeIds.has(r.id) &&
      (q === "" || r.file_name.toLowerCase().includes(q) || r.device_name.toLowerCase().includes(q)),
  );

  return (
    <div>
      <h1 className="mb-6 text-xl font-semibold">Transfer History</h1>
      {error && <p className="mb-4 text-sm text-red-400">{error}</p>}

      <ActiveTransferList variant="card" />

      <input
        type="search"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder="Search by file or device"
        className="mb-6 w-full rounded-lg border border-tovi-border bg-tovi-panel px-3 py-2 text-sm text-slate-100 placeholder-slate-500 focus:border-tovi-blue focus:outline-none"
      />

      {loading ? (
        <p className="text-slate-400">Loading…</p>
      ) : visible.length === 0 ? (
        <div className="flex flex-col items-center gap-3 pt-20 text-slate-400">
          <span className="text-5xl">📋</span>
          <p>
            {q !== "" ? "No matches" : activeIds.size > 0 ? "Nothing finished yet" : "No transfers yet"}
          </p>
        </div>
      ) : (
        groupByDate(visible).map(([label, items]) => (
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
                  {r.status === "completed" && r.path && (
                    <div className="flex shrink-0 gap-2">
                      <button
                        onClick={() => openFile(r, false)}
                        className="rounded-lg bg-tovi-blue px-3 py-1 text-xs text-white hover:bg-blue-500"
                      >
                        Open
                      </button>
                      <button
                        onClick={() => openFile(r, true)}
                        className="rounded-lg border border-tovi-border px-3 py-1 text-xs text-slate-200 hover:bg-tovi-border"
                      >
                        Show in folder
                      </button>
                    </div>
                  )}
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
