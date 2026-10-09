import TransferProgress from "./TransferProgress";
import { useActiveTransfers } from "../useActiveTransfers";

/**
 * Live rows for every transfer in flight. Its own component so only it
 * re-renders on each progress event, not the page around it.
 */
export default function ActiveTransferList({ variant }: { variant: "card" | "compact" }) {
  const active = useActiveTransfers();
  if (active.length === 0) return null;

  if (variant === "compact") {
    return (
      <div className="space-y-3">
        {active.map((t) => (
          <div key={t.id}>
            <p className="mb-1 truncate text-sm text-slate-200">
              {t.direction === "sent" ? "↑" : "↓"} {t.fileName} · {t.deviceName}
            </p>
            <TransferProgress done={t.done} total={t.total} speed={t.speed} reconnecting={t.reconnecting} />
          </div>
        ))}
      </div>
    );
  }

  return (
    <section className="mb-6">
      <h2 className="mb-2 text-xs font-semibold uppercase tracking-wider text-slate-500">
        In progress
      </h2>
      <ul className="space-y-2">
        {active.map((t) => (
          <li key={t.id} className="rounded-xl border border-tovi-blue/40 bg-tovi-panel px-5 py-3">
            <p className="truncate font-medium text-slate-100">{t.fileName}</p>
            <p className="mb-2 text-sm text-slate-400">
              {t.direction === "sent" ? "↑ To" : "↓ From"} {t.deviceName}
            </p>
            <TransferProgress done={t.done} total={t.total} speed={t.speed} reconnecting={t.reconnecting} />
          </li>
        ))}
      </ul>
    </section>
  );
}
