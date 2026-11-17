import { Component, type ReactNode } from "react";
import { diagnosticsLogRendererError } from "../lib/native";
import { Button } from "../ui/Button";
import { PageTitle } from "../ui/Heading";
import { XFalconMark } from "../ui/XFalconMark";

export interface AppErrorBoundaryProps {
  children: ReactNode;
  onReset?: () => void;
  reload?: () => void;
}

interface AppErrorBoundaryState {
  failed: boolean;
}

export class AppErrorBoundary extends Component<AppErrorBoundaryProps, AppErrorBoundaryState> {
  state: AppErrorBoundaryState = { failed: false };

  static getDerivedStateFromError(): AppErrorBoundaryState {
    return { failed: true };
  }

  componentDidCatch(error: Error, errorInfo: { componentStack?: string | null }): void {
    void diagnosticsLogRendererError({
      version: 1,
      message: error?.name ? `${error.name}: ${error.message || "render failure"}` : "render failure",
      component_stack: errorInfo?.componentStack ?? null,
    }).catch(() => {
      // Silence any IPC failures so recovery surface remains interactive
    });
  }

  private reset = (): void => {
    if (this.props.onReset) {
      this.props.onReset();
      this.setState({ failed: false });
      return;
    }

    (this.props.reload ?? (() => window.location.reload()))();
  };

  render(): ReactNode {
    if (!this.state.failed) return this.props.children;

    return (
      <main className="app-recovery" aria-labelledby="app-recovery-title">
        <section role="alert" className="app-recovery__panel">
          <span aria-hidden className="app-mark">
            <XFalconMark />
          </span>
          <PageTitle level={1} id="app-recovery-title">
            xFalcon needs to recover
          </PageTitle>
          <p className="type-body text-secondary">
            The workspace could not be displayed. Private error details were not shown.
          </p>
          <Button className="mt-4 h-10" type="button" variant="primary" onClick={this.reset}>
            Reload workspace
          </Button>
        </section>
      </main>
    );
  }
}
