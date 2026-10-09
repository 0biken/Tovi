import { NavLink, Outlet, useLocation } from "react-router-dom";
import Prompts from "./Prompts";
import TransferProgress from "./TransferProgress";
import { useActiveTransfers } from "../useActiveTransfers";

const navItems = [
  { to: "/", label: "Devices", icon: "📡" },
  { to: "/pair", label: "Pair", icon: "📷" },
  { to: "/history", label: "History", icon: "📋" },
  { to: "/settings", label: "Settings", icon: "⚙️" },
];

export default function Layout() {
  const active = useActiveTransfers();
  // Send and History show progress themselves
  const { pathname } = useLocation();
  const showFooter =
    active.length > 0 && !pathname.startsWith("/send") && !pathname.startsWith("/history");
  return (
    <div className="flex h-screen w-screen overflow-hidden bg-tovi-dark">
      {/* Sidebar */}
      <aside className="flex w-16 flex-col items-center gap-6 border-r border-tovi-border bg-tovi-panel py-6">
        <span className="text-2xl font-bold text-tovi-blue">T</span>
        <nav className="flex flex-col items-center gap-4 mt-4">
          {navItems.map((item) => (
            <NavLink
              key={item.to}
              to={item.to}
              end={item.to === "/"}
              title={item.label}
              aria-label={item.label}
              className={({ isActive }) =>
                `flex h-10 w-10 items-center justify-center rounded-lg text-lg transition-colors ${
                  isActive
                    ? "bg-tovi-blue text-white"
                    : "text-slate-400 hover:bg-tovi-border hover:text-slate-100"
                }`
              }
            >
              {item.icon}
            </NavLink>
          ))}
        </nav>
      </aside>

      {/* Main content */}
      <div className="flex min-w-0 flex-1 flex-col">
        <main className="flex-1 overflow-y-auto p-6">
          <Outlet />
        </main>
        {showFooter && (
          <footer className="space-y-3 border-t border-tovi-border bg-tovi-panel px-6 py-3">
            {active.map((t) => (
              <div key={t.id}>
                <p className="mb-1 truncate text-sm text-slate-200">
                  {t.direction === "sent" ? "↑" : "↓"} {t.fileName} · {t.deviceName}
                </p>
                <TransferProgress
                  done={t.done}
                  total={t.total}
                  speed={t.speed}
                  reconnecting={t.reconnecting}
                />
              </div>
            ))}
          </footer>
        )}
      </div>

      <Prompts />
    </div>
  );
}
