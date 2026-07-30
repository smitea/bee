import React from "react";

import { auditRecord } from "../ipc/audit";

interface Props {
  children: React.ReactNode;
  fallback?: (err: Error, reset: () => void) => React.ReactNode;
  label?: string;
}

interface State {
  hasError: boolean;
  error: Error | null;
}

export class ErrorBoundary extends React.Component<Props, State> {
  state: State = { hasError: false, error: null };

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo): void {
    console.error("ErrorBoundary caught error", error, info);
    Promise.resolve(
      auditRecord({
        actor: "bee-client",
        action: "ui.error",
        result: "Failure",
        summary: `${this.props.label ?? "ui"}: ${error.message ?? "unknown error"}`,
      }),
    ).catch(() => undefined);
  }

  reset = (): void => {
    this.setState({ hasError: false, error: null });
  };

  render(): React.ReactNode {
    if (this.state.hasError && this.state.error) {
      if (this.props.fallback) {
        return this.props.fallback(this.state.error, this.reset);
      }
      return (
        <div
          className="flex flex-col items-center justify-center gap-3 p-6 text-xs text-gray-600 dark:text-neutral-300"
          role="alert"
          data-testid="error-boundary-fallback"
        >
          <div className="font-medium text-accent-red">
            {this.props.label ? `${this.props.label} crashed` : "Something went wrong"}
          </div>
          <div className="font-mono text-[10px] text-gray-500 max-w-md break-words text-center">
            {this.state.error.message}
          </div>
          <button
            type="button"
            onClick={this.reset}
            className="px-3 py-1 text-[10px] rounded border border-gray-300 dark:border-neutral-600 hover:bg-gray-100 dark:hover:bg-neutral-700"
            data-testid="error-boundary-reset"
          >
            Reset
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}