import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { api, Device, subscribe } from "../api";
import { platformIcon, timeAgo } from "../format";

export default function DeviceListPage() {
  const [devices, setDevices] = useState<Device[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  /** Device whose Forget button was clicked once and awaits confirmation */
  const [confirming, setConfirming] = useState<string | null>(null);
  const navigate = useNavigate();

  const refresh = useCallback(async () => {
    try {
      setDevices(await api.listDevices());
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
    return subscribe("devices:changed", refresh);
  }, [refresh]);

  const forget = async (device: Device) => {
    if (confirming !== device.id) {
      setConfirming(device.id);
      return;
    }
    setConfirming(null);
    try {
      await api.forgetDevice(device.id);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div>
      <div className="mb-6 flex items-center justify-between">
        <h1 className="text-xl font-semibold">Devices</h1>
        <button
          onClick={() => navigate("/pair")}
          className="rounded-lg bg-tovi-panel px-4 py-2 text-sm text-slate-300 hover:bg-tovi-border transition-colors"
        >
          Pair a device
        </button>
      </div>

      {error && <p className="mb-4 text-sm text-red-400">{error}</p>}

      {loading ? (
        <p className="text-slate-400">Loading…</p>
      ) : devices.length === 0 ? (
        <div className="flex flex-col items-center gap-4 pt-20 text-slate-400">
          <span className="text-5xl">📡</span>
          <p className="text-lg">No paired devices yet</p>
          <p className="text-sm">Pair once with a QR code; after that it's one click.</p>
          <button
            onClick={() => navigate("/pair")}
            className="mt-2 rounded-lg bg-tovi-blue px-6 py-2 text-sm font-medium text-white hover:bg-blue-500 transition-colors"
          >
            Connect via QR
          </button>
        </div>
      ) : (
        <ul className="space-y-3">
          {devices.map((device) => (
            <li
              key={device.id}
              className="flex items-center gap-4 rounded-xl border border-tovi-border bg-tovi-panel px-5 py-4"
            >
              <span className="text-3xl">{platformIcon(device.platform)}</span>
              <div className="min-w-0 flex-1">
                <p className="truncate font-medium text-slate-100">{device.name}</p>
                <p className="text-sm text-slate-400">
                  {device.platform}
                  {device.last_seen ? ` · seen ${timeAgo(device.last_seen)}` : ""}
                  {device.address ? ` · ${device.address}` : ""}
                </p>
              </div>
              <button
                onClick={() => forget(device)}
                onBlur={() => setConfirming(null)}
                className={`rounded-lg px-3 py-2 text-sm transition-colors ${
                  confirming === device.id
                    ? "bg-red-600 text-white hover:bg-red-500"
                    : "text-slate-400 hover:bg-tovi-border hover:text-slate-100"
                }`}
              >
                {confirming === device.id ? "Click to confirm" : "Forget"}
              </button>
              <button
                onClick={() => navigate(`/send/${device.id}`)}
                className="rounded-lg bg-tovi-blue px-4 py-2 text-sm font-medium text-white hover:bg-blue-500 transition-colors"
              >
                Send
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
