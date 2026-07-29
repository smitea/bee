import { useEffect } from "react";
import { useStore } from "./state/store";
import { AppBar } from "./components/AppBar";
import { AppShell } from "./components/AppShell";
import { StatusBar } from "./components/StatusBar";
import { Dashboard } from "./pages/Dashboard";
import { DataSources } from "./pages/DataSources";
import { Pipelines } from "./pages/Pipelines";
import { Settings } from "./pages/Settings";

export default function App() {
  const tab = useStore((s) => s.tab);
  const theme = useStore((s) => s.theme);

  useEffect(() => {
    document.documentElement.classList.toggle("dark", theme === "dark");
  }, [theme]);

  const body = (() => {
    switch (tab) {
      case "dashboard":
        return <Dashboard />;
      case "dataSources":
        return <DataSources />;
      case "pipelines":
        return <Pipelines />;
      case "settings":
        return <Settings />;
    }
  })();

  return <AppShell>{body}</AppShell>;
}

// keep AppBar in scope for the existing icon re-exports
void AppBar;
void StatusBar;