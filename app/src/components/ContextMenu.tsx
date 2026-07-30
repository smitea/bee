import { useEffect, useRef } from "react";

export interface ContextMenuItem {
  id: string;
  label: string;
  onSelect(): void;
  disabled?: boolean;
}

interface Props {
  open: boolean;
  items: ContextMenuItem[];
  onClose(): void;
  x?: number;
  y?: number;
}

export function ContextMenu({ open, items, onClose, x = 0, y = 0 }: Props) {
  const ref = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, onClose]);

  if (!open) return null;
  return (
    <div
      ref={ref}
      role="menu"
      data-testid="context-menu"
      className="fixed z-50 min-w-[10rem] rounded-md border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-800 shadow-lg py-1 text-xs"
      style={{ left: x, top: y }}
    >
      {items.map((it) => (
        <button
          key={it.id}
          role="menuitem"
          type="button"
          disabled={it.disabled}
          onClick={() => {
            if (it.disabled) return;
            it.onSelect();
            onClose();
          }}
          className={[
            "block w-full text-left px-3 py-1.5",
            it.disabled
              ? "text-gray-400 dark:text-neutral-500 cursor-not-allowed"
              : "text-gray-700 dark:text-neutral-200 hover:bg-gray-100 dark:hover:bg-neutral-700",
          ].join(" ")}
        >
          {it.label}
        </button>
      ))}
    </div>
  );
}