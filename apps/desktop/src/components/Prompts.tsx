import { useEffect, useState } from "react";
import { api, subscribe } from "../api";
import { formatBytes, platformIcon, shortId } from "../format";

/** Something waiting for the user's yes / no */
type Request =
  | {
      kind: "pairing";
      requestId: number;
      deviceId: string;
      deviceName: string;
      platform: string;
    }
  | {
      kind: "file";
      requestId: number;
      deviceName: string;
      fileName: string;
      fileSize: number;
    };

interface Toast {
  id: number;
  text: string;
  tone: "ok" | "error";
}

let nextToast = 1;

/**
 * App-wide prompts (pairing requests, incoming files when auto-accept is off)
 * and short notifications. Lives in the layout so it shows on every screen.
 */
export default function Prompts() {
  const [requests, setRequests] = useState<Request[]>([]);
  const [toasts, setToasts] = useState<Toast[]>([]);

  const toast = (text: string, tone: Toast["tone"] = "ok") => {
    const id = nextToast++;
    setToasts((t) => [...t, { id, text, tone }]);
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), 5000);
  };
  const dismiss = (requestId: number) =>
    setRequests((r) => r.filter((x) => x.requestId !== requestId));

  useEffect(() => {
    const stops = [
      subscribe("pairing:request", (e) =>
        setRequests((r) => [
          ...r,
          {
            kind: "pairing",
            requestId: e.request_id,
            deviceId: e.device_id,
            deviceName: e.device_name,
            platform: e.platform,
          },
        ]),
      ),
      subscribe("transfer:incoming", (e) =>
        setRequests((r) => [
          ...r,
          {
            kind: "file",
            requestId: e.request_id,
            deviceName: e.device_name,
            fileName: e.file_name,
            fileSize: e.file_size,
          },
        ]),
      ),
      subscribe("request:expired", (e) => {
        dismiss(e.request_id);
        toast("No answer in time, so the request was declined", "error");
      }),
      subscribe("pairing:done", (e) => toast(`Paired with ${e.device_name}`)),
      subscribe("transfer:finished", (e) => {
        if (e.direction !== "received") return;
        if (e.status === "completed") {
          toast(`Saved ${e.file_name} from ${e.device_name}`);
        } else if (e.status === "interrupted") {
          toast(`${e.file_name} from ${e.device_name} was interrupted; it will resume`, "error");
        } else {
          toast(`Could not receive ${e.file_name}: ${e.error ?? "unknown error"}`, "error");
        }
      }),
    ];
    return () => stops.forEach((stop) => stop());
  }, []);

  const answer = async (request: Request, allow: boolean) => {
    dismiss(request.requestId);
    const accepted = await api.respond(request.requestId, allow);
    if (!accepted) toast("That request had already expired", "error");
  };

  const current = requests[0];

  return (
    <>
      {current && (
        <div className="fixed inset-0 z-40 flex items-center justify-center bg-black/60">
          <div
            role="dialog"
            aria-modal="true"
            aria-labelledby="prompt-title"
            className="w-full max-w-sm rounded-2xl border border-tovi-border bg-tovi-panel p-6 shadow-2xl"
          >
            {current.kind === "pairing" ? (
              <>
                <p className="text-4xl">{platformIcon(current.platform)}</p>
                <h2 id="prompt-title" className="mt-3 text-lg font-semibold">
                  {current.deviceName} wants to pair
                </h2>
                <p className="mt-1 text-sm text-slate-400">
                  {current.platform} · Device ID {shortId(current.deviceId)}
                </p>
                <p className="mt-3 text-sm text-slate-300">
                  Only allow devices you recognise. Once paired, it can send you files.
                </p>
              </>
            ) : (
              <>
                <p className="text-4xl">📥</p>
                <h2 id="prompt-title" className="mt-3 text-lg font-semibold">
                  {current.deviceName} wants to send a file
                </h2>
                <p className="mt-1 truncate text-sm text-slate-300">{current.fileName}</p>
                <p className="text-sm text-slate-400">{formatBytes(current.fileSize)}</p>
              </>
            )}
            <div className="mt-6 flex justify-end gap-3">
              <button
                onClick={() => answer(current, false)}
                className="rounded-lg px-4 py-2 text-sm text-slate-300 hover:bg-tovi-border"
              >
                {current.kind === "pairing" ? "Cancel" : "Decline"}
              </button>
              <button
                autoFocus
                onClick={() => answer(current, true)}
                className="rounded-lg bg-tovi-blue px-4 py-2 text-sm font-medium text-white hover:bg-blue-500"
              >
                {current.kind === "pairing" ? "Allow" : "Accept"}
              </button>
            </div>
            {requests.length > 1 && (
              <p className="mt-3 text-xs text-slate-500">{requests.length - 1} more waiting</p>
            )}
          </div>
        </div>
      )}

      <div className="fixed bottom-4 right-4 z-50 flex w-80 flex-col gap-2" aria-live="polite">
        {toasts.map((t) => (
          <div
            key={t.id}
            className={`rounded-lg border px-4 py-3 text-sm shadow-lg ${
              t.tone === "ok"
                ? "border-tovi-border bg-tovi-panel text-slate-100"
                : "border-red-900 bg-red-950 text-red-100"
            }`}
          >
            {t.text}
          </div>
        ))}
      </div>
    </>
  );
}
