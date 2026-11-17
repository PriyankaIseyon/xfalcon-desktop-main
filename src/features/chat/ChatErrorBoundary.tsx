import { Component, type ReactNode } from "react";
import { diagnosticsLogRendererError } from "../../lib/native";
import { Alert } from "../../ui/Alert";
import { Button } from "../../ui/Button";

export type ChatErrorBoundaryProps = {
  children: ReactNode;
  onReset?: () => void;
  /** A different conversation is a fresh render surface. One broken chat must
   * not hold every later conversation behind its fallback. */
  resetKey?: string | null;
};

type ChatErrorBoundaryState = {
  failed: boolean;
  failures: number;
  resetKey: string | null | undefined;
};

export class ChatErrorBoundary extends Component<ChatErrorBoundaryProps, ChatErrorBoundaryState> {
  state: ChatErrorBoundaryState = { failed: false, failures: 0, resetKey: this.props.resetKey };

  static getDerivedStateFromProps(
    props: ChatErrorBoundaryProps,
    state: ChatErrorBoundaryState,
  ): Partial<ChatErrorBoundaryState> | null {
    if (props.resetKey === state.resetKey) return null;
    // Reset before rendering the new conversation. componentDidUpdate would
    // leave the old fallback visible for one committed frame first.
    return { failed: false, failures: 0, resetKey: props.resetKey };
  }

  static getDerivedStateFromError(): Partial<ChatErrorBoundaryState> {
    return { failed: true };
  }

  componentDidCatch(error: Error, errorInfo: { componentStack?: string | null }): void {
    this.setState((state) => ({ failures: state.failures + 1 }));
    void diagnosticsLogRendererError({
      version: 1,
      message: error?.name ? `${error.name}: ${error.message || "render failure"}` : "render failure",
      component_stack: errorInfo?.componentStack ?? null,
    }).catch(() => {
      // Silence any IPC failures so recovery surface remains interactive
    });
  }

  private reset = (): void => {
    this.props.onReset?.();
    this.setState({ failed: false });
  };

  render(): ReactNode {
    if (!this.state.failed) return this.props.children;

    return (
      <section aria-label="Chat recovery" className="flex flex-1 flex-col justify-center px-6 py-8">
        <Alert
          action={
            <Button size="sm" variant="secondary" onClick={this.reset}>
              Try again
            </Button>
          }
          className="mx-auto w-full max-w-2xl"
          title={this.state.failures > 1 ? "Chat still could not be displayed" : "Chat could not be displayed"}
          tone="danger"
        >
          <p className="type-caption">
            The rest of the workspace is still available. Message content was not included in this notice.
          </p>
        </Alert>
      </section>
    );
  }
}
