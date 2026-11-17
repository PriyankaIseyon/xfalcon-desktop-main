import React from "react";
import { AppShell } from "./app/AppShell";
import { AuthGate } from "./features/auth/AuthGate";
import { setAuthSession } from "./features/auth/authStore";
import type { AppBootstrap } from "./lib/generated/contracts";
import { appBootstrap } from "./lib/native";
import { startBackgroundUpdateChecks } from "./features/settings/updateStore";
import { Button } from "./ui/Button";
import { Spinner } from "./ui/Spinner";
import { Toaster } from "./ui/Toaster";
import { PageTitle } from "./ui/Heading";
import { XFalconMark } from "./ui/XFalconMark";
import { setPlatform } from "./lib/platform";

type BootstrapState =
  { status: "loading" } | { status: "ready"; data: AppBootstrap } | { status: "error"; message: string };

export default function App() {
  const [bootstrapState, setBootstrapState] = React.useState<BootstrapState>({
    status: "loading",
  });
  const [bootstrapAttempt, setBootstrapAttempt] = React.useState(0);

  React.useEffect(() => {
    let active = true;
    let stopUpdateChecks: (() => void) | null = null;

    appBootstrap().then(
      (data) => {
        if (active) {
          document.documentElement.dataset.platform = data.platform;
          // The stylesheet reads the attribute; `Shortcut` reads the store.
          setPlatform(data.platform);
          setAuthSession(data.session);
          setBootstrapState({ status: "ready", data });
          void startBackgroundUpdateChecks().then((stop) => {
            if (active) stopUpdateChecks = stop;
            else stop();
          });
        }
      },
      () => {
        if (active) {
          setBootstrapState({
            status: "error",
            message: "The secure native runtime could not be reached.",
          });
        }
      },
    );

    return () => {
      active = false;
      stopUpdateChecks?.();
    };
  }, [bootstrapAttempt]);

  const retryBootstrap = () => {
    setBootstrapState({ status: "loading" });
    setBootstrapAttempt((attempt) => attempt + 1);
  };

  if (bootstrapState.status === "loading") {
    return (
      <main aria-labelledby="startup-title" className="app-startup">
        <div>
          <span aria-hidden className="app-mark">
            <XFalconMark />
          </span>
          <PageTitle level={1} id="startup-title">
            xFalcon Desktop
          </PageTitle>
          <p
            role="status"
            aria-live="polite"
            className="type-body text-secondary flex items-center justify-center gap-2"
          >
            <Spinner decorative size={18} />
            Starting xFalcon…
          </p>
        </div>
      </main>
    );
  }

  if (bootstrapState.status === "error") {
    return (
      <main aria-labelledby="startup-error-title" className="app-startup">
        <div role="alert">
          <span aria-hidden className="app-mark">
            <XFalconMark />
          </span>
          <PageTitle level={1} id="startup-error-title">
            Unable to start xFalcon
          </PageTitle>
          <p className="type-body text-secondary">{bootstrapState.message}</p>
          <Button size="lg" className="mt-4" type="button" variant="primary" onClick={retryBootstrap}>
            Try again
          </Button>
        </div>
      </main>
    );
  }

  return (
    <>
      <AuthGate bootstrap={bootstrapState.data}>
        <AppShell bootstrap={bootstrapState.data} />
      </AuthGate>

      <Toaster />
    </>
  );
}
