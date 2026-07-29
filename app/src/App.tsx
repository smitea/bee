import { useEffect } from "react";
import { useUi } from "./state/store";
import { AppShell } from "./components/AppShell";
import { useConnection } from "./state/connectionStore";
import { connState } from "./ipc";

export default function App() {
  const theme = useUi((s) => s.theme);
  const addr = useConnection((s) => s.addr);
  const setStatus = useConnection((s) => s.setStatus);

  useEffect(() => {
    document.documentElement.classList.toggle("dark", theme === "dark");
  }, [theme]);

  useEffect(() => {
    let cancelled = false;
    const tick = async () => {
      try {
        const view = await connState(addr);
        if (!cancelled) setStatus({ addr: view.addr, status: view.status });
      } catch {
        if (!cancelled) setStatus({ kind: "Disconnected" });
      }
    };
    void tick();
    const handle = setInterval(tick, 4000);
    return () => {
      cancelled = true;
      clearInterval(handle);
    };
  }, [addr, setStatus]);

  return <AppShell />;
}