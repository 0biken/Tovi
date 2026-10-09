import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "../api";

export default function SettingsPage() {
  const [me, setMe] = useState<{ device_id: string; device_name: string } | null>(null);
  const [folder, setFolder] = useState("");
  const [autoAccept, setAutoAccept] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    Promise.all([api.getLocalId(), api.getReceiveFolder(), api.getAutoAccept()])
      .then(([id, receive, auto]) => {
        setMe(id);
        setFolder(receive.path);
        setAutoAccept(auto);
      })
      .catch((e) => setError(String(e)));
  }, []);

  const chooseFolder = async () => {
    const selected = await open({ directory: true, defaultPath: folder || undefined });
    if (typeof selected !== "string") return;
    try {
      await api.setReceiveFolder(selected);
      setFolder((await api.getReceiveFolder()).path);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  };

  const toggleAutoAccept = async () => {
    const next = !autoAccept;
    try {
      await api.setAutoAccept(next);
      setAutoAccept(next);
    } catch (e) {
      setError(String(e));
    }
  };

  const copyId = async () => {
    if (!me) return;
    await navigator.clipboard.writeText(me.device_id);
    setCopied(true);
  };

  return (
    <div className="max-w-2xl">
      <h1 className="mb-6 text-xl font-semibold">Settings</h1>
      {error && <p className="mb-4 text-sm text-red-400">{error}</p>}

      <section className="mb-4 rounded-xl border border-tovi-border bg-tovi-panel p-5">
        <h2 className="text-sm font-semibold text-slate-200">Receive folder</h2>
        <p className="mt-1 text-sm text-slate-400">Files from other devices are saved here.</p>
        <div className="mt-3 flex items-center gap-2">
          <code className="min-w-0 flex-1 truncate rounded-lg bg-tovi-dark px-3 py-2 text-sm text-slate-300">
            {folder || "…"}
          </code>
          <button
            onClick={chooseFolder}
            className="shrink-0 rounded-lg bg-tovi-border px-4 py-2 text-sm text-slate-100 hover:bg-slate-600"
          >
            Change…
          </button>
        </div>
      </section>

      <section className="mb-4 flex items-start justify-between gap-6 rounded-xl border border-tovi-border bg-tovi-panel p-5">
        <div>
          <h2 id="auto-accept-label" className="text-sm font-semibold text-slate-200">
            Accept files from paired devices automatically
          </h2>
          <p className="mt-1 text-sm text-slate-400">
            When off, you'll be asked before each file is saved. New devices always need your
            approval to pair.
          </p>
        </div>
        <button
          role="switch"
          aria-checked={autoAccept}
          aria-labelledby="auto-accept-label"
          onClick={toggleAutoAccept}
          className={`relative h-6 w-11 shrink-0 rounded-full transition-colors ${
            autoAccept ? "bg-tovi-blue" : "bg-tovi-border"
          }`}
        >
          <span
            className={`absolute top-0.5 h-5 w-5 rounded-full bg-white transition-all ${
              autoAccept ? "left-[22px]" : "left-0.5"
            }`}
          />
        </button>
      </section>

      <section className="rounded-xl border border-tovi-border bg-tovi-panel p-5">
        <h2 className="text-sm font-semibold text-slate-200">This device</h2>
        <p className="mt-1 text-sm text-slate-300">{me?.device_name ?? "…"}</p>
        <div className="mt-3 flex items-center gap-2">
          <code className="min-w-0 flex-1 truncate rounded-lg bg-tovi-dark px-3 py-2 text-xs text-slate-400">
            {me?.device_id ?? "…"}
          </code>
          <button
            onClick={copyId}
            className="shrink-0 rounded-lg bg-tovi-border px-4 py-2 text-sm text-slate-100 hover:bg-slate-600"
          >
            {copied ? "Copied" : "Copy ID"}
          </button>
        </div>
      </section>
    </div>
  );
}
