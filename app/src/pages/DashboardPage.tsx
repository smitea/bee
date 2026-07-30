import { DashboardEditor } from "../components/DashboardEditor";
import { useApplications } from "../state/applicationsStore";

interface Props {
  applicationId: number;
}

export function DashboardPage({ applicationId }: Props) {
  const applications = useApplications((s) => s.items);
  const application = applications.find((a) => a.id === applicationId);
  return (
    <div className="space-y-4">
      <header className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold">
            Dashboard{application ? ` · ${application.name}` : ""}
          </h1>
          <p className="text-xs text-gray-500 dark:text-neutral-400">
            drag, resize, and reorder panels. Layout persists per application.
          </p>
        </div>
      </header>
      <DashboardEditor applicationId={applicationId} />
    </div>
  );
}
