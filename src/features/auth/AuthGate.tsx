import type { ReactNode } from "react";
import type { AppBootstrap } from "../../lib/generated/contracts";
import { authCancel } from "../../lib/native";
import { Button } from "../../ui/Button";
import { PageTitle } from "../../ui/Heading";
import { Spinner } from "../../ui/Spinner";
import { XFalconMark } from "../../ui/XFalconMark";
import { toast } from "../../ui/toastStore";
import { setAuthSession } from "./authStore";
import { LoginView } from "./LoginView";
import { isAuthenticated, useAuthSession } from "./useAuth";

export interface AuthGateProps {
  bootstrap: AppBootstrap;
  children: ReactNode;
}

function platformLabel(platform: AppBootstrap["platform"]): string {
  const names: Record<AppBootstrap["platform"], string> = {
    windows: "Windows",
    macos: "macOS",
    linux: "Linux",
    unknown: "Desktop",
  };
  return names[platform];
}

export function AuthGate({ bootstrap, children }: AuthGateProps) {
  const session = useAuthSession();

  if (session.status === "authenticating") {
    /**
     * Reachable without ever touching `LoginView`'s own `loading` state: the
     * backend holds "authenticating" across a restart while its loopback
     * listener is still open, so bootstrap can land here directly. Carries
     * the same mark-plus-card frame as the other startup surfaces instead of
     * a bare spinner, and its own cancel -- `LoginView`'s cancel button is
     * local UI state that does not exist yet on this mount, so without one
     * here a person who relaunched mid sign-in had no way back to the login
     * surface short of waiting out the five-minute loopback hold.
     */
    return (
      <main aria-labelledby="authenticating-title" className="app-startup">
        <div>
          <span aria-hidden className="app-mark">
            <XFalconMark />
          </span>
          <PageTitle level={1} id="authenticating-title">
            Continuing sign-in…
          </PageTitle>
          <p
            role="status"
            aria-live="polite"
            className="type-body text-secondary flex items-center justify-center gap-2"
          >
            <Spinner decorative size={18} />
            Opening secure sign-in…
          </p>
          <Button
            size="lg"
            className="mt-4 w-full"
            type="button"
            variant="secondary"
            // A rejected cancel must not become an unhandled rejection with a
            // dead button beside it. The hold expires on its own, so reporting
            // it and leaving this surface in place is the honest outcome.
            onClick={() =>
              void authCancel().then(setAuthSession, () => {
                toast.error("Sign-in could not be cancelled. It will time out on its own.");
              })
            }
          >
            Cancel sign-in
          </Button>
        </div>
      </main>
    );
  }

  if (!isAuthenticated(session)) {
    return <LoginView platformLabel={`${platformLabel(bootstrap.platform)} · ${bootstrap.application_version}`} />;
  }

  return children;
}
