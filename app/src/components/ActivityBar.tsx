import { useEffect, useState } from "react";
import { Activity, ChevronUp } from "lucide-react";

import { useAudit, summary } from "../state/auditStore";
import { ActivityDialog } from "./ActivityDialog";

export function ActivityBar() {
  const events = useAudit((s) => s.events);
  const loaded = useAudit((s) => s.loaded);
  const refresh = useAudit((s) => s.refresh);
  const latest = useAudit((s) => s.latest);

  const [open, setOpen] = useState(false);

  useEffect(() => {
    if (!loaded) void refresh(50);
  }, [loaded, refresh]);

  const top = events[0];
  const text = top ? summary(top) : "No activity yet";

  return (
    <>
      <button
        type="button"
        onClick={() => {
          void latest();
          setOpen(true);
        }}
        className="flex items-center gap-1.5 text-[10px] text-gray-600 dark:text-neutral-300 hover:text-accent-blue"
        title="Open activity"
        aria-label="Open activity"
      >
        <Activity size={11} />
        <span className="max-w-[28rem] truncate">{text}</span>
        <ChevronUp size={10} />
      </button>
      {open && <ActivityDialog onClose={() => setOpen(false)} />}
    </>
  );
}