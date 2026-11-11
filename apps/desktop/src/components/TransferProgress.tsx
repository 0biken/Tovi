import { formatBytes, formatDuration, formatSpeed } from "../format";

interface Props {
  done: number;
  total: number;
  speed: number;
  reconnecting?: boolean;
}

/** Progress bar with percent, bytes, live speed and time remaining */
export default function TransferProgress({ done, total, speed, reconnecting }: Props) {
  const percent = total > 0 ? Math.min(100, (done / total) * 100) : 0;
  const eta = speed > 0 && total > done ? (total - done) / speed : null;

  return (
    <div className="w-full">
      <div
        className="h-2 w-full overflow-hidden rounded-full bg-slate-700"
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(percent)}
      >
        <div
          className={`h-full rounded-full transition-all duration-300 ${
            reconnecting ? "bg-amber-400" : "bg-tovi-blue"
          }`}
          style={{ width: `${percent}%` }}
        />
      </div>
      <p className="mt-1 flex justify-between text-xs text-slate-400">
        <span>
          {reconnecting
            ? "Connection lost. Reconnecting…"
            : total > 0
              ? `${Math.round(percent)}% · ${formatBytes(done)} of ${formatBytes(total)}`
              : "Connecting…"}
        </span>
        {!reconnecting && speed > 0 && (
          <span>
            {formatSpeed(speed)}
            {eta !== null ? ` · ${formatDuration(eta)} left` : ""}
          </span>
        )}
      </p>
    </div>
  );
}
