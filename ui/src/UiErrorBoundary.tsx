import { Component, type ErrorInfo, type ReactNode } from "react";

type UiErrorBoundaryProps = {
  children: ReactNode;
  scope: string;
  resetKey?: string;
  className?: string;
};

type UiErrorBoundaryState = {
  error: string | null;
};

export class UiErrorBoundary extends Component<UiErrorBoundaryProps, UiErrorBoundaryState> {
  state: UiErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: unknown): UiErrorBoundaryState {
    return { error: uiErrorMessage(error) };
  }

  componentDidCatch(error: unknown, info: ErrorInfo) {
    console.error(`[cadastrophe] ${this.props.scope} render failed`, {
      error,
      componentStack: info.componentStack
    });
  }

  componentDidUpdate(previousProps: UiErrorBoundaryProps) {
    if (previousProps.resetKey !== this.props.resetKey && this.state.error) {
      this.setState({ error: null });
    }
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <section
        className={this.props.className ?? "ui-error-boundary"}
        data-testid={`${this.props.scope.toLowerCase()}-error-boundary`}
        role="alert"
      >
        <strong>{this.props.scope} render failed</strong>
        <span>{this.state.error}</span>
      </section>
    );
  }
}

function uiErrorMessage(error: unknown): string {
  if (error instanceof Error && error.message.trim()) return error.message;
  if (typeof error === "string" && error.trim()) return error;
  return "Unknown UI rendering error.";
}
