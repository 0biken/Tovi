import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { QRCodeSVG } from "qrcode.react";
import { api, QrPayload } from "../api";

export default function QrPairingPage() {
  const [qr, setQr] = useState<QrPayload | null>(null);
  const [secondsLeft, setSecondsLeft] = useState(60);
  const [qrError, setQrError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const [code, setCode] = useState("");
  const [pairing, setPairing] = useState(false);
  const [pairError, setPairError] = useState<string | null>(null);
  const navigate = useNavigate();

  const generate = useCallback(async () => {
    try {
      const payload = await api.generateQr();
      setQr(payload);
      setQrError(null);
      setCopied(false);
      setSecondsLeft(Math.max(0, payload.expires_at - Math.floor(Date.now() / 1000)));
    } catch (e) {
      setQr(null);
      setQrError(String(e));
    }
  }, []);

  useEffect(() => {
    generate();
  }, [generate]);

  // Count down; a fresh code replaces the old one when it runs out
  useEffect(() => {
    if (!qr) return;
    const tick = setInterval(() => {
      setSecondsLeft((s) => {
        if (s <= 1) {
          generate();
          return 0;
        }
        return s - 1;
      });
    }, 1000);
    return () => clearInterval(tick);
  }, [qr, generate]);

  const copy = async () => {
    if (!qr) return;
    await navigator.clipboard.writeText(qr.payload);
    setCopied(true);
  };

  const pairWithCode = async (e: React.FormEvent) => {
    e.preventDefault();
    setPairing(true);
    setPairError(null);
    try {
      const device = await api.pairWithCode(code.trim());
      navigate(`/send/${device.id}`);
    } catch (err) {
      setPairError(String(err));
    } finally {
      setPairing(false);
    }
  };

  return (
    <div className="mx-auto flex max-w-xl flex-col items-center gap-6 pt-6">
      <h1 className="text-xl font-semibold">Pair a device</h1>
      <p className="text-sm text-slate-400">Scan with TOVI on your phone, or paste this code into another TOVI.</p>

      <div className="rounded-2xl border border-tovi-border bg-white p-6 shadow-2xl">
        {qr ? (
          <QRCodeSVG value={qr.payload} size={200} bgColor="#ffffff" fgColor="#0f172a" />
        ) : (
          <div className="flex h-[200px] w-[200px] items-center justify-center p-2 text-center text-sm text-slate-500">
            {qrError ?? "Generating…"}
          </div>
        )}
      </div>

      {qr && (
        <>
          <div className="flex items-center gap-2 text-sm text-slate-400">
            <span
              className={`font-mono text-lg font-bold ${
                secondsLeft < 10 ? "text-red-400" : "text-slate-200"
              }`}
            >
              {secondsLeft}s
            </span>
            <span>until a new code</span>
          </div>
          <div className="flex w-full items-center gap-2">
            <code className="min-w-0 flex-1 truncate rounded-lg bg-tovi-panel px-3 py-2 text-xs text-slate-400">
              {qr.payload}
            </code>
            <button
              onClick={copy}
              className="shrink-0 rounded-lg bg-tovi-panel px-4 py-2 text-sm text-slate-300 hover:bg-tovi-border"
            >
              {copied ? "Copied" : "Copy code"}
            </button>
          </div>
        </>
      )}
      {qrError && (
        <button onClick={generate} className="text-sm text-tovi-blue hover:underline">
          Try again
        </button>
      )}

      <form onSubmit={pairWithCode} className="mt-6 w-full border-t border-tovi-border pt-6">
        <label htmlFor="pair-code" className="text-sm font-medium text-slate-200">
          Pair with a code from another device
        </label>
        <div className="mt-2 flex gap-2">
          <input
            id="pair-code"
            value={code}
            onChange={(e) => setCode(e.target.value)}
            placeholder="tovi://pair/…"
            spellCheck={false}
            className="min-w-0 flex-1 rounded-lg border border-tovi-border bg-tovi-panel px-3 py-2 text-sm text-slate-100 placeholder:text-slate-500 focus:border-tovi-blue focus:outline-none"
          />
          <button
            type="submit"
            disabled={pairing || !code.trim()}
            className="shrink-0 rounded-lg bg-tovi-blue px-4 py-2 text-sm font-medium text-white hover:bg-blue-500 disabled:opacity-50"
          >
            {pairing ? "Pairing…" : "Pair"}
          </button>
        </div>
        {pairing && (
          <p className="mt-2 text-sm text-slate-400">Waiting for the other device to allow it…</p>
        )}
        {pairError && <p className="mt-2 text-sm text-red-400">{pairError}</p>}
      </form>
    </div>
  );
}
