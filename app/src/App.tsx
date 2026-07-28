import { useEffect } from "react";
import { useStore } from "./state/store";
import { AppBar } from "./components/AppBar";
import { StatusBar } from "./components/StatusBar";
import { Dashboard } from "./pages/Dashboard";
import { DataSources } from "./pages/DataSources";
import { Pipelines } from "./pages/Pipelines";
import { Settings } from "./pages/Settings";

export default function App() {
  const tab = useStore((s) => s.tab);
  const theme = useStore((s) => s.theme);

  // Apply dark class to <html> based on theme
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

  return (
    <div className="h-full flex flex-col">
      <AppBar />
      <main className="flex-1 overflow-auto p-6 bg-gray-50 dark:bg-neutral-900">
        {body}
      </main>
      <StatusBar />
    </div>
  );
}