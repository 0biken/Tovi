import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { api, Device, subscribe } from "../api";
import { formatBytes, formatSpeed, platformIcon } from "../format";

type State = "idle" | "drag-over" | "transferring" | "done" | "error";

export default function SendPage() {
  const { deviceId } = useParams<{ deviceId: string }>();
  const navigate = useNavigate();
  const [device, setDevice] = useState<Device | null>(null);
  const [state, setState] = useState<State>("idle");
  const [error, setError] = useState<string | null>(null);

  // Progress of the file being sent now
  const [fileName, setFileName] = useState("");
  const [queue, setQueue] = useState({ index: 0, count: 0 });
  const [progress, setProgress] = useState({ done: 0, total: 0, speed: 0 });
  const [reconnecting, setReconnecting] = useState(false);
  const [sent, setSent] = useState<string[]>([]);
  /** The transfer this page is showing, learned from `transfer:started` */
  const currentTransfer = useRef<string | null>(null);
  const busy = useRef(false);

  useEffect(() => {
    api
      .listDevices()
      .then((all) => setDevice(all.find((d) => d.id === deviceId) ?? null))
      .catch((e) => setError(String(e)));
  }, [deviceId]);

  // Follow this page's transfer through the backend's events
  useEffect(() => {
    const stops = [
      subscribe("transfer:started", (e) => {
        if (busy.current && e.direction === "sent" && e.device_id === deviceId) {
          currentTransfer.current = e.transfer_id;
        }
      }),
      subscribe("transfer:progress", (e) => {
        if (e.transfer_id !== currentTransfer.current) return;
        setReconnecting(false);
        setProgress({ done: e.done, total: e.total, speed: e.bytes_per_sec });
      }),
      subscribe("transfer:reconnecting", (e) => {
        if (e.transfer_id === currentTransfer.current) setReconnecting(true);
      }),
    ];
    return () => stops.forEach((stop) => stop());
  }, [deviceId]);

  const sendFiles = useCallback(
    async (paths: string[]) => {
      if (!deviceId || paths.length === 0 || busy.current) return;
      busy.current = true;
      setState("transferring");
      setError(null);
      setSent([]);
      try {
        for (const [index, path] of paths.entries()) {
          const name = path.split(/[\\/]/).pop() ?? path;
          currentTransfer.current = null;
          setFileName(name);
          setQueue({ index: index + 1, count: paths.length });
          setProgress({ done: 0, total: 0, speed: 0 });
          setReconnecting(false);
          await api.sendFile(deviceId, path);
          setSent((s) => [...s, name]);
        }
        setState("done");
      } catch (e) {
        setError(String(e));
        setState("error");
      } finally {
        busy.current = false;
      }
    },
    [deviceId],
  );

  // Drag and drop: Tauri gives real file paths through the webview event
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    getCurrentWebview()
      .onDragDropEvent((event) => {
        if (busy.current) return;
        const payload = event.payload;
        if (payload.type === "enter" || payload.type === "over") setState("drag-over");
        else if (payload.type === "leave") setState((s) => (s === "drag-over" ? "idle" : s));
        else if (payload.type === "drop") sendFiles(payload.paths);
      })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [sendFiles]);

  const browse = async () => {
    const selected = await open({ multiple: true, directory: false });
    if (selected) sendFiles(Array.isArray(selected) ? selected : [selected]);
  };

  const percent = progress.total > 0 ? Math.round((progress.done / progress.total) * 100) : 0;

  if (!device && !error) {
    return <p className="text-slate-400">Loading…</p>;
  }
  if (!device) {
    return (
      <div className="flex flex-col items-start gap-4">
        <p className="text-red-400">This device is no longer paired.</p>
        <button onClick={() => navigate("/")} className="text-sm text-tovi-blue hover:underline">
          Back to devices
        </button>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col items-center justify-center gap-8">
      <button
        onClick={() => navigate("/")}
        className="self-start text-sm text-slate-400 hover:text-slate-100"
      >
        ← Back
      </button>

      <div className="flex items-center gap-3">
        <span className="text-3xl">{platformIcon(device.platform)}</span>
        <div>
          <p className="text-sm text-slate-400">Send to</p>
          <p className="font-semibold">{device.name}</p>
        </div>
      </div>

      {(state === "idle" || state === "drag-over") && (
        <button
          onClick={browse}
          className={`flex h-72 w-full max-w-lg flex-col items-center justify-center gap-4 rounded-2xl border-2 border-dashed transition-all ${
            state === "drag-over"
              ? "border-tovi-blue bg-blue-950/40 scale-[1.02]"
              : "border-tovi-border bg-tovi-panel hover:border-tovi-blue"
          }`}
        >
          <span className="text-5xl">{state === "drag-over" ? "📂" : "📁"}</span>
          <span className="text-lg font-medium text-slate-200">
            {state === "drag-over" ? "Release to send" : "Drop files here"}
          </span>
          <span className="text-sm text-slate-400">or click to browse</span>
        </button>
      )}

      {state === "transferring" && (
        <div className="flex flex-col items-center gap-4">
          <svg className="h-32 w-32 -rotate-90" viewBox="0 0 100 100" aria-hidden="true">
            <circle cx="50" cy="50" r="40" stroke="#334155" strokeWidth="10" fill="none" />
            <circle
              cx="50"
              cy="50"
              r="40"
              stroke="#3B82F6"
              strokeWidth="10"
              fill="none"
              strokeDasharray={`${2 * Math.PI * 40}`}
              strokeDashoffset={`${2 * Math.PI * 40 * (1 - percent / 100)}`}
              strokeLinecap="round"
              className="transition-all duration-300"
            />
          </svg>
          <p className="text-3xl font-bold" aria-live="polite">
            {percent}%
          </p>
          <p className="max-w-sm truncate text-sm text-slate-300">
            {fileName}
            {queue.count > 1 ? ` (${queue.index} of ${queue.count})` : ""}
          </p>
          <p className="text-sm text-slate-400">
            {reconnecting
              ? "Connection lost. Reconnecting to resume…"
              : progress.total > 0
                ? `${formatBytes(progress.done)} of ${formatBytes(progress.total)} · ${formatSpeed(progress.speed)}`
                : "Connecting…"}
          </p>
        </div>
      )}

      {state === "done" && (
        <div className="flex flex-col items-center gap-4">
          <span className="text-6xl">✅</span>
          <p className="text-xl font-semibold text-green-400">
            {sent.length === 1 ? "File delivered" : `${sent.length} files delivered`}
          </p>
          <p className="text-sm text-slate-400">Verified by {device.name}</p>
          <button onClick={() => setState("idle")} className="text-sm text-tovi-blue hover:underline">
            Send more
          </button>
        </div>
      )}

      {state === "error" && (
        <div className="flex max-w-lg flex-col items-center gap-4 text-center">
          <span className="text-6xl">❌</span>
          <p className="text-xl font-semibold text-red-400">Transfer failed</p>
          {sent.length > 0 && (
            <p className="text-sm text-slate-400">Delivered before the failure: {sent.join(", ")}</p>
          )}
          <p className="text-sm text-slate-400">{error}</p>
          <button
            onClick={() => setState("idle")}
            className="rounded-lg bg-tovi-blue px-6 py-2 text-sm text-white hover:bg-blue-500"
          >
            Try again
          </button>
        </div>
      )}
    </div>
  );
}
