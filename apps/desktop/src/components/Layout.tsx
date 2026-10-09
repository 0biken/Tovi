import { NavLink, Outlet, useLocation } from "react-router-dom";
import Prompts from "./Prompts";
import ActiveTransferList from "./ActiveTransferList";
import { useActiveTransferIds } from "../useActiveTransfers";

const navItems = [
  { to: "/", label: "Devices", icon: "📡" },
  { to: "/pair", label: "Pair", icon: "📷" },
  { to: "/history", label: "History", icon: "📋" },
  { to: "/settings", label: "Settings", icon: "⚙️" },
];

export default function Layout() {
  const activeIds = useActiveTransferIds();
  // Send and History show progress themselves
  const { pathname } = useLocation();
  const showFooter =
    activeIds.size > 0 && !pathname.startsWith("/send") && !pathname.startsWith("/history");
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
          <footer className="border-t border-tovi-border bg-tovi-panel px-6 py-3">
            <ActiveTransferList variant="compact" />
          </footer>
        )}
      </div>

      <Prompts />
    </div>
  );
}
