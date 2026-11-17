import { useRef, useState } from "react";
import { ExternalLink } from "lucide-react";
import { Alert } from "../../ui/Alert";
import { Button } from "../../ui/Button";
import { EmptyState } from "../../ui/EmptyState";
import { HeroTitle } from "../../ui/Heading";
import { Logo } from "../../ui/Logo";
import { authCancel, authLogin } from "../../lib/native";
import { setAuthSession } from "./authStore";
import { describeLoginError, loginErrorOf } from "./authPresentation";
import { useSignedOutNotice } from "./signedOutNotice";

export interface LoginViewProps {
  platformLabel: string;
}

/**
 * The sign-in surface.
 *
 * A refusal is shown as the reason for it, from the code the command rejects
 * with. A single sentence for every cause sends someone to their administrator
 * for a port that is busy on their own machine, and leaves nothing in the log
 * either — the backend now records the operator detail, and this shows the
 * person their half.
 *
 * Waiting is cancellable. The browser flow holds a loopback port for five
 * minutes, and a person who closed the browser tab has no other way to get the
 * port back: without a cancel, every later attempt is refused as already
 * authenticating until the deadline passes.
 */
export function LoginView({ platformLabel }: LoginViewProps) {
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const cancelledHere = useRef(false);
  const signedOutNotice = useSignedOutNotice();

  const signIn = async () => {
    setError(null);
    setLoading(true);
    cancelledHere.current = false;
    try {
      const session = await authLogin();
      setAuthSession(session);
    } catch (cause) {
      if (cancelledHere.current) return;
      const code = loginErrorOf(cause);
      setError(
        code === null ? "Sign-in is unavailable. Contact the administrator, then try again." : describeLoginError(code),
      );
    } finally {
      setLoading(false);
    }
  };

  const cancel = async () => {
    cancelledHere.current = true;
    setError(null);
    setLoading(false);
    setAuthSession(await authCancel());
  };

  return (
    <main aria-labelledby="login-title" className="app-startup">
      <div className="w-full max-w-md">
        <Logo showWordmark subtitle={platformLabel} className="mb-6" />
        <EmptyState
          description="Sign-in completes in your default browser, then returns you here."
          title={<HeroTitle id="login-title">Sign in to xFalcon</HeroTitle>}
          actions={
            <div className="w-full space-y-2">
              {signedOutNotice ? (
                <Alert tone="info" title="Signed out" className="text-left">
                  {signedOutNotice}
                </Alert>
              ) : null}
              <Button size="lg" className="w-full" loading={loading} type="button" variant="primary" onClick={signIn}>
                {loading ? (
                  "Waiting for browser…"
                ) : (
                  <>
                    Continue in browser
                    <ExternalLink aria-hidden size={16} strokeWidth={1.8} />
                  </>
                )}
              </Button>
              {loading ? (
                <Button size="lg" className="w-full" type="button" variant="secondary" onClick={() => void cancel()}>
                  Cancel sign-in
                </Button>
              ) : null}
            </div>
          }
        />
        {error ? (
          <p className="type-caption mt-4 text-danger" role="alert">
            {error}
          </p>
        ) : null}
      </div>
    </main>
  );
}
