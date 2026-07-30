import { useEffect } from "react";
import { Activity, ChevronUp } from "lucide-react";

import { useAudit, summary } from "../state/auditStore";
import { useTabs } from "../state/tabsStore";

export interface ActivityBarProps {
  navigate?: (kind: string, resourceId: string | null) => void;
}

export function ActivityBar({ navigate }: ActivityBarProps = {}) {
  const events = useAudit((s) => s.events);
  const loaded = useAudit((s) => s.loaded);
  const refresh = useAudit((s) => s.refresh);
  const latest = useAudit((s) => s.latest);
  const openActivity = useTabs((s) => s.openActivity);

  useEffect(() => {
    if (!loaded) void refresh(50);
  }, [loaded, refresh]);

  const top = events[0];
  const text = top ? summary(top) : "No activity yet";

  const onOpenActivity = () => {
    void latest();
    if (navigate) {
      navigate("activity", null);
      return;
    }
    void openActivity();
  };

  return (
    <button
      type="button"
      onClick={onOpenActivity}
      className="flex items-center gap-1.5 text-[10px] text-gray-600 dark:text-neutral-300 hover:text-accent-blue"
      title="Click for full activity"
      aria-label="Click for full activity"
    >
      <Activity size={11} />
      <span className="max-w-[28rem] truncate">{text}</span>
      <ChevronUp size={10} />
    </button>
  );
}
