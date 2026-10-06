import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { api, Device } from "../api";

const PLATFORM_ICONS: Record<string, string> = {
  macos: "🍎",
  windows: "🪟",
  linux: "🐧",
  android: "🤖",
  ios: "📱",
};

export default function DeviceListPage() {
  const [devices, setDevices] = useState<Device[]>([]);
  const [loading, setLoading] = useState(true);
  const navigate = useNavigate();

  const refresh = async () => {
    try {
      const found = await api.listDevices();
      setDevices(found);
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    refresh();
    const interval = setInterval(refresh, 2000);
    return () => clearInterval(interval);
  }, []);

  return (
    <div>
      <div className="mb-6 flex items-center justify-between">
        <h1 className="text-xl font-semibold">Nearby Devices</h1>
        <button
          onClick={refresh}
          className="rounded-lg bg-tovi-panel px-4 py-2 text-sm text-slate-300 hover:bg-tovi-border transition-colors"
        >
          Refresh
        </button>
      </div>

      {loading ? (
        <p className="text-slate-400">Scanning local network…</p>
      ) : devices.length === 0 ? (
        <div className="flex flex-col items-center gap-4 pt-20 text-slate-400">
          <span className="text-5xl">📡</span>
          <p className="text-lg">No devices found</p>
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
            <li key={device.id}>
              <button
                onClick={() => navigate(`/send/${device.id}`)}
                className="flex w-full items-center gap-4 rounded-xl border border-tovi-border bg-tovi-panel px-5 py-4 text-left transition-colors hover:border-tovi-blue hover:bg-slate-800"
              >
                <span className="text-3xl">
                  {PLATFORM_ICONS[device.platform] ?? "💻"}
                </span>
                <div>
                  <p className="font-medium text-slate-100">{device.name}</p>
                  <p className="text-sm text-slate-400">
                    {device.platform} · {device.address}:{device.port} · Local
                  </p>
                </div>
                <span className="ml-auto text-tovi-blue">→</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
