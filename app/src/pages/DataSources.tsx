import { Database } from "lucide-react";

export function DataSources() {
  return (
    <div className="space-y-6">
      <h1 className="text-xl font-semibold">Data Sources</h1>
      <div className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700 p-12 text-center">
        <Database
          size={64}
          className="mx-auto text-gray-300 dark:text-neutral-600"
        />
        <p className="mt-4 text-gray-600 dark:text-neutral-300 font-medium">
          Data Sources
        </p>
        <p className="mt-2 text-sm text-gray-500 dark:text-neutral-400">
          This feature will be implemented in S-Tauri.2
        </p>
      </div>
    </div>
  );
}