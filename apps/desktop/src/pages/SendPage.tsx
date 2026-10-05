import { useCallback, useState } from "react";
import { useParams, useNavigate } from "react-router-dom";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "../api";

type State = "idle" | "drag-over" | "transferring" | "done" | "error";

interface ProgressPayload {
  bytes_sent: number;
  total_bytes: number;
  speed_bps: number;
}

export default function SendPage() {
  const { deviceId } = useParams<{ deviceId: string }>();
  const navigate = useNavigate();
  const [state, setState] = useState<State>("idle");
  const [progress, setProgress] = useState(0);
  const [speed, setSpeed] = useState(0);
  const [error, setError] = useState<string | null>(null);

  const sendFile = useCallback(async (filePath: string) => {
    if (!deviceId) return;
    setState("transferring");
    setProgress(0);

    const unlisten = await listen<ProgressPayload>("transfer:progress", (event) => {
      const { bytes_sent, total_bytes, speed_bps } = event.payload;
      setProgress(Math.round((bytes_sent / total_bytes) * 100));
      setSpeed(speed_bps);
    });

    try {
      await api.sendFile(deviceId, filePath);
      setState("done");
    } catch (e) {
      setError(String(e));
      setState("error");
    } finally {
      unlisten();
    }
  }, [deviceId]);

  const onDrop = async (e: React.DragEvent) => {
    e.preventDefault();
    setState("idle");
    const file = e.dataTransfer.files[0];
    if (file) {
      // Tauri exposes the real path via the webkitRelativePath or via dialog
      await sendFile((file as any).path ?? file.name);
    }
  };

  const onClickBrowse = async () => {
    const selected = await open({ multiple: false, directory: false });
    if (typeof selected === "string") await sendFile(selected);
  };

  const fmt = (bps: number) =>
    bps > 1_000_000 ? `${(bps / 1_000_000).toFixed(1)} MB/s` : `${(bps / 1000).toFixed(0)} KB/s`;

  return (
    <div className="flex h-full flex-col items-center justify-center gap-8">
      <button
        onClick={() => navigate("/")}
        className="self-start text-sm text-slate-400 hover:text-slate-100"
      >
        ← Back
      </button>

      {/* Drop Zone */}
      {(state === "idle" || state === "drag-over") && (
        <div
          onDragOver={(e) => { e.preventDefault(); setState("drag-over"); }}
          onDragLeave={() => setState("idle")}
          onDrop={onDrop}
          onClick={onClickBrowse}
          className={`flex h-72 w-full max-w-lg cursor-pointer flex-col items-center justify-center gap-4 rounded-2xl border-2 border-dashed transition-all ${
            state === "drag-over"
              ? "border-tovi-blue bg-blue-950/40 scale-[1.02]"
              : "border-tovi-border bg-tovi-panel hover:border-tovi-blue"
          }`}
        >
          <span className="text-5xl">{state === "drag-over" ? "📂" : "📁"}</span>
          <p className="text-lg font-medium text-slate-200">
            {state === "drag-over" ? "Release to send" : "DROP FILES HERE"}
          </p>
          <p className="text-sm text-slate-400">or click to browse</p>
        </div>
      )}

      {/* Progress */}
      {state === "transferring" && (
        <div className="flex flex-col items-center gap-4">
          <svg className="h-32 w-32 -rotate-90" viewBox="0 0 100 100">
            <circle cx="50" cy="50" r="40" stroke="#334155" strokeWidth="10" fill="none" />
            <circle
              cx="50" cy="50" r="40"
              stroke="#3B82F6" strokeWidth="10" fill="none"
              strokeDasharray={`${2 * Math.PI * 40}`}
              strokeDashoffset={`${2 * Math.PI * 40 * (1 - progress / 100)}`}
              strokeLinecap="round"
              className="transition-all duration-300"
            />
          </svg>
          <p className="text-3xl font-bold">{progress}%</p>
          <p className="text-sm text-slate-400">{fmt(speed)}</p>
        </div>
      )}

      {/* Done */}
      {state === "done" && (
        <div className="flex flex-col items-center gap-4">
          <span className="text-6xl">✅</span>
          <p className="text-xl font-semibold text-green-400">File delivered</p>
          <button onClick={() => setState("idle")} className="text-sm text-tovi-blue hover:underline">
            Send another
          </button>
        </div>
      )}

      {/* Error */}
      {state === "error" && (
        <div className="flex flex-col items-center gap-4">
          <span className="text-6xl">❌</span>
          <p className="text-xl font-semibold text-red-400">Transfer failed</p>
          <p className="text-sm text-slate-400">{error}</p>
          <button onClick={() => setState("idle")} className="rounded-lg bg-tovi-blue px-6 py-2 text-sm text-white hover:bg-blue-500">
            Retry
          </button>
        </div>
      )}
    </div>
  );
}
