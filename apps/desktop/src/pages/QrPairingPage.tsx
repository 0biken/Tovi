import { useEffect, useState, useCallback } from "react";
import { QRCodeSVG } from "qrcode.react";
import { api, QrPayload } from "../api";

export default function QrPairingPage() {
  const [qr, setQr] = useState<QrPayload | null>(null);
  const [secondsLeft, setSecondsLeft] = useState(60);
  const [loading, setLoading] = useState(true);

  const generate = useCallback(async () => {
    setLoading(true);
    try {
      const payload = await api.generateQr();
      setQr(payload);
      const remaining = Math.max(
        0,
        payload.expires_at - Math.floor(Date.now() / 1000)
      );
      setSecondsLeft(remaining);
    } finally {
      setLoading(false);
    }
  }, []);

  // Auto-generate on mount
  useEffect(() => { generate(); }, [generate]);

  // Countdown timer — auto-refresh when it hits 0
  useEffect(() => {
    if (!qr) return;
    const tick = setInterval(() => {
      setSecondsLeft((s) => {
        if (s <= 1) { generate(); return 0; }
        return s - 1;
      });
    }, 1000);
    return () => clearInterval(tick);
  }, [qr, generate]);

  return (
    <div className="flex flex-col items-center justify-center gap-8 pt-12">
      <h1 className="text-xl font-semibold">Connect a Phone</h1>
      <p className="text-sm text-slate-400">Scan with TOVI on your phone</p>

      <div className="rounded-2xl border border-tovi-border bg-white p-6 shadow-2xl">
        {loading || !qr ? (
          <div className="flex h-48 w-48 items-center justify-center text-slate-400">
            Generating…
          </div>
        ) : (
          <QRCodeSVG value={qr.payload} size={200} bgColor="#ffffff" fgColor="#0f172a" />
        )}
      </div>

      {/* Countdown */}
      <div className="flex items-center gap-2 text-sm text-slate-400">
        <span
          className={`font-mono text-lg font-bold ${
            secondsLeft < 10 ? "text-red-400" : "text-slate-200"
          }`}
        >
          {secondsLeft}s
        </span>
        <span>until QR refreshes</span>
      </div>

      <button
        onClick={generate}
        className="rounded-lg bg-tovi-panel px-5 py-2 text-sm text-slate-300 hover:bg-tovi-border transition-colors"
      >
        Refresh now
      </button>
    </div>
  );
}
