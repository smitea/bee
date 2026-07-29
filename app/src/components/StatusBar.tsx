import { useStore } from "../state/store";

export function StatusBar() {
  const theme = useStore((s) => s.theme);
  const addr = useStore((s) => s.addr);
  return (
    <footer className="flex items-center gap-4 px-6 h-7 text-[10px] text-gray-500 dark:text-neutral-400 bg-white dark:bg-neutral-800 border-t border-gray-200 dark:border-neutral-700">
      <span>bee-gui v0.1.0 (Tauri)</span>
      <span>addr: {addr}</span>
      <span>theme: {theme}</span>
    </footer>
  );
}