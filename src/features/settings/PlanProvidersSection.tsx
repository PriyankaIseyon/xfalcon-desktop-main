import { useCallback, useEffect, useId, useRef, useState, type ReactNode, type RefObject } from "react";
import { Copy } from "lucide-react";
import type {
  ChatgptPlanState,
  ChatgptPlanStatusView,
  ClaudeEffort,
  ClaudeRuntimeStatusView,
} from "../../lib/generated/contracts";
import { writeClipboardText } from "../chat/clipboard";
import { invalidateModelCatalog } from "../models/useModelCatalog";
import { Alert } from "../../ui/Alert";
import { AlertDialog } from "../../ui/AlertDialog";
import { Badge } from "../../ui/Badge";
import { Button } from "../../ui/Button";
import { DialogShell } from "../../ui/DialogShell";
import { SectionTitle } from "../../ui/Heading";
import { Link } from "../../ui/Link";
import { Select } from "../../ui/Select";
import { Spinner } from "../../ui/Spinner";
import { Switch } from "../../ui/Switch";
import { toast } from "../../ui/toastStore";
import { SettingsButton } from "./SettingsControls";
import { SettingsGroup, SettingsGroupHeader, SettingsRow } from "./SettingsLayout";
import { CHATGPT_PLAN_COPY, CLAUDE_RUNTIME_COPY } from "./settingsCopy";
import { useChatgptPlan, useClaudeRuntime } from "./usePlanProviders";

// The select lists the levels in the order the copy defines them.
const CLAUDE_EFFORT_OPTIONS = (Object.keys(CLAUDE_RUNTIME_COPY.effortLevels) as ClaudeEffort[]).map((value) => ({
  value,
  label: CLAUDE_RUNTIME_COPY.effortLevels[value].label,
}));

export interface PlanProvidersSectionProps {
  /**
   * Called after a change that can alter the model catalog: the Claude switch,
   * a sign-in or sign-out, or a re-check. The shell re-reads the workspace from it.
   */
  onWorkspaceChanged?: () => void;
}

export function PlanProvidersSection({ onWorkspaceChanged }: PlanProvidersSectionProps) {
  // Both cards report through here, so every provider change also reloads open model pickers.
  const onChanged = () => {
    invalidateModelCatalog();
    onWorkspaceChanged?.();
  };
  return (
    <SettingsGroup id="settings-plan-providers" aria-label="Plan providers">
      <ClaudeRuntimeCard onChanged={onChanged} />
      <ChatgptPlanCard onChanged={onChanged} />
    </SettingsGroup>
  );
}

/**
 * Puts focus on a card's control after a change that replaced or disabled the
 * control the reader was using. The returned function is called before the
 * change starts. Focus moves once the card has settled, so it never lands on a
 * busy card or while a dialog is open. Focus the reader moved elsewhere is left
 * where it is.
 *
 * WKWebView does not focus a button on click, and a button that is removed or
 * disabled drops focus, so the focus is moved explicitly rather than left to
 * the browser.
 */
function useFocusAfterChange(cardRef: RefObject<HTMLElement | null>, selector: string, settled: boolean) {
  const requested = useRef(false);

  // Runs after every render, not only when `settled` changes: a command whose
  // busy state never renders as true would otherwise leave the request pending.
  useEffect(() => {
    if (!requested.current || !settled) return;
    requested.current = false;
    const active = document.activeElement;
    if (active && active !== document.body && !cardRef.current?.contains(active)) return;
    cardRef.current?.querySelector<HTMLElement>(selector)?.focus();
  });

  return useCallback(() => {
    requested.current = true;
  }, []);
}

function ClaudeRuntimeCard({ onChanged }: { onChanged?: () => void }) {
  const headingId = useId();
  const cardRef = useRef<HTMLElement | null>(null);
  const runtime = useClaudeRuntime();
  const [consentOpen, setConsentOpen] = useState(false);
  const status = runtime.status;
  // The switch stays in place through every change, so it is the card's control to return to.
  const focusSwitch = useFocusAfterChange(cardRef, '[role="switch"]', !runtime.busy && !consentOpen);
  const effortLabelId = useId();
  const effortHintId = useId();
  // The effort select is disabled while its save runs, which drops focus, so focus returns to it once the save settles.
  const focusEffort = useFocusAfterChange(cardRef, `[aria-labelledby="${effortLabelId}"]`, !runtime.busy);

  // The dialog closes on failure too: the failure is then shown on the card.
  const enable = async () => {
    focusSwitch();
    const saved = await runtime.setEnabled(true);
    setConsentOpen(false);
    if (saved) onChanged?.();
  };

  const disable = async () => {
    focusSwitch();
    if (await runtime.setEnabled(false)) onChanged?.();
  };

  // The dialog returns focus to whatever had it when it opened. WKWebView does
  // not focus a switch on click, so that return lands nowhere: move it here.
  const cancelConsent = () => {
    focusSwitch();
    setConsentOpen(false);
  };

  const checkAgain = async () => {
    if (await runtime.refresh()) onChanged?.();
  };

  // A level changes only chats started afterwards, not the model catalog, so no onChanged here.
  const chooseEffort = (effort: ClaudeEffort) => {
    focusEffort();
    void runtime.setEffort(effort);
  };

  const copyCommand = async (command: string) => {
    try {
      await writeClipboardText(command);
      toast.success(CLAUDE_RUNTIME_COPY.commandCopied);
    } catch {
      toast.error(CLAUDE_RUNTIME_COPY.commandCopyFailed);
    }
  };

  const checkAgainButton = <CheckAgainButton busy={runtime.busy} onCheck={() => void checkAgain()} />;

  return (
    <section aria-labelledby={headingId} className="settings-collection-item" ref={cardRef}>
      <SettingsGroupHeader
        headingId={headingId}
        title={CLAUDE_RUNTIME_COPY.title}
        description={CLAUDE_RUNTIME_COPY.description}
      >
        <Switch
          busy={runtime.busy}
          checked={status?.enabled ?? false}
          disabled={status === null}
          label={CLAUDE_RUNTIME_COPY.switchLabel}
          onChange={(next) => {
            if (next) setConsentOpen(true);
            else void disable();
          }}
        />
      </SettingsGroupHeader>

      {runtime.error ? <Alert tone="danger">{runtime.error}</Alert> : null}
      {status === null ? (
        runtime.busy ? (
          <LoadingLine text={CLAUDE_RUNTIME_COPY.loading} />
        ) : (
          <StateLine action={checkAgainButton} />
        )
      ) : null}
      {status?.enabled ? (
        <ClaudeRuntimeDetail
          status={status}
          onCopyCommand={(command) => void copyCommand(command)}
          checkAgain={checkAgainButton}
        />
      ) : null}
      {status?.enabled ? (
        <SettingsRow
          label={CLAUDE_RUNTIME_COPY.effortLabel}
          hint={
            <span id={effortHintId}>
              {`${CLAUDE_RUNTIME_COPY.effortLevels[status.effort].description} ${CLAUDE_RUNTIME_COPY.effortNote}`}
            </span>
          }
          labelId={effortLabelId}
        >
          <Select
            aria-describedby={effortHintId}
            aria-labelledby={effortLabelId}
            className="w-full sm:w-64"
            disabled={runtime.busy}
            onChange={chooseEffort}
            options={CLAUDE_EFFORT_OPTIONS}
            value={status.effort}
          />
        </SettingsRow>
      ) : null}

      <AlertDialog
        busy={runtime.busy}
        cancelLabel={CLAUDE_RUNTIME_COPY.cancel}
        confirmLabel={CLAUDE_RUNTIME_COPY.enable}
        description={CLAUDE_RUNTIME_COPY.consentDescription}
        onCancel={cancelConsent}
        onConfirm={() => void enable()}
        open={consentOpen}
        title={CLAUDE_RUNTIME_COPY.consentTitle}
      >
        <ul className="list-disc space-y-2 pl-5">
          {CLAUDE_RUNTIME_COPY.consentPoints.map((point) => (
            <li key={point}>{point}</li>
          ))}
        </ul>
      </AlertDialog>
    </section>
  );
}

interface ClaudeRuntimeDetailProps {
  status: ClaudeRuntimeStatusView;
  checkAgain: ReactNode;
  onCopyCommand: (command: string) => void;
}

function ClaudeRuntimeDetail({ status, checkAgain, onCopyCommand }: ClaudeRuntimeDetailProps) {
  switch (status.state) {
    case "disabled":
      return null;
    case "not_installed":
      return <StateLine action={checkAgain}>{CLAUDE_RUNTIME_COPY.notInstalled}</StateLine>;
    case "outdated":
      return (
        <StateLine action={checkAgain}>
          {CLAUDE_RUNTIME_COPY.outdated(status.installed_version, status.required_version)}
        </StateLine>
      );
    case "signed_out":
      return (
        <StateLine action={checkAgain}>
          <span>{CLAUDE_RUNTIME_COPY.signedOut}</span>
          <div className="flex w-full flex-wrap items-center gap-2">
            <code className="min-w-0 flex-1 break-all rounded-md border border-hairline bg-surface px-2 py-1 font-mono type-micro text-foreground">
              {status.sign_in_command}
            </code>
            <SettingsButton icon={<Copy />} onClick={() => onCopyCommand(status.sign_in_command)}>
              {CLAUDE_RUNTIME_COPY.copyCommand}
            </SettingsButton>
          </div>
        </StateLine>
      );
    case "ready": {
      const readyText = [
        CLAUDE_RUNTIME_COPY.ready,
        status.plan ? CLAUDE_RUNTIME_COPY.plan(status.plan) : null,
        status.sign_in_method ? CLAUDE_RUNTIME_COPY.signInMethod(status.sign_in_method) : null,
      ]
        .filter((part): part is string => part !== null)
        .join(" ");
      return <StateLine>{readyText}</StateLine>;
    }
    case "check_failed":
      return <StateLine action={checkAgain}>{CLAUDE_RUNTIME_COPY.checkFailed}</StateLine>;
  }
}

function CheckAgainButton({ busy, onCheck }: { busy: boolean; onCheck: () => void }) {
  return (
    <SettingsButton disabled={busy} onClick={onCheck}>
      {CLAUDE_RUNTIME_COPY.checkAgain}
    </SettingsButton>
  );
}

function ChatgptPlanCard({ onChanged }: { onChanged?: () => void }) {
  const headingId = useId();
  const welcomeTitleId = useId();
  const welcomeBodyId = useId();
  const cardRef = useRef<HTMLElement | null>(null);
  const plan = useChatgptPlan();
  const status = plan.status;
  const welcomeOpen = status?.state === "ready" && status.welcome_pending;
  // Focus waits while the notice is open. The sign-in that opened it keeps the
  // request, so focus moves to the card once the notice closes.
  const settled = !plan.busy && !plan.signingIn && !welcomeOpen;
  const focusControl = useFocusAfterChange(cardRef, "[data-plan-focus]", settled);
  // A status can carry its own failure text: a sign-in that failed while ready,
  // or a sign-out that OpenAI did not confirm. It shows once: in the notice while
  // the notice is open, and on the card otherwise.
  const statusError = status?.error ? <Alert tone="danger">{status.error}</Alert> : null;

  const signIn = async () => {
    focusControl();
    if (await plan.signIn()) onChanged?.();
  };

  const signOut = async () => {
    focusControl();
    if (await plan.signOut()) onChanged?.();
  };

  const refresh = async () => {
    if (await plan.refresh()) onChanged?.();
  };

  return (
    <section aria-labelledby={headingId} className="settings-collection-item border-t border-hairline" ref={cardRef}>
      <SettingsGroupHeader
        headingId={headingId}
        title={CHATGPT_PLAN_COPY.title}
        description={CHATGPT_PLAN_COPY.description}
      />

      {plan.error && !welcomeOpen ? <Alert tone="danger">{plan.error}</Alert> : null}
      {!welcomeOpen ? statusError : null}
      <ChatgptPlanDetail
        busy={plan.busy}
        signingIn={plan.signingIn}
        status={status}
        onCancelSignIn={() => void plan.cancelSignIn()}
        onRetry={() => void refresh()}
        onSignIn={() => void signIn()}
        onSignOut={() => void signOut()}
      />

      {/* A notice the reader must answer: no scrim or Escape dismissal, so it
          cannot be waved away without being acknowledged. */}
      <DialogShell
        className="surface-card xf-material xf-overlay-pop alert-panel flex w-full max-w-md flex-col gap-4 p-5 shadow-e3"
        describedBy={welcomeBodyId}
        labelledBy={welcomeTitleId}
        open={welcomeOpen}
      >
        <SectionTitle className="shrink-0 break-words" id={welcomeTitleId} level={2}>
          {CHATGPT_PLAN_COPY.welcomeTitle}
        </SectionTitle>
        <div className="flex flex-col gap-4">
          <p className="type-body text-secondary" id={welcomeBodyId}>
            {CHATGPT_PLAN_COPY.welcomeBody}
          </p>
          {plan.error ? <Alert tone="danger">{plan.error}</Alert> : null}
          {statusError}
        </div>
        <div className="flex justify-end pt-2">
          <Button
            data-dialog-initial-focus=""
            disabled={plan.busy}
            loading={plan.busy}
            type="button"
            variant="primary"
            onClick={() => void plan.acknowledgeWelcome()}
          >
            {CHATGPT_PLAN_COPY.welcomeAction}
          </Button>
        </div>
      </DialogShell>
    </section>
  );
}

interface ChatgptPlanDetailProps {
  status: ChatgptPlanStatusView | null;
  busy: boolean;
  signingIn: boolean;
  onSignIn: () => void;
  onCancelSignIn: () => void;
  onSignOut: () => void;
  onRetry: () => void;
}

function ChatgptPlanDetail({
  status,
  busy,
  signingIn,
  onSignIn,
  onCancelSignIn,
  onSignOut,
  onRetry,
}: ChatgptPlanDetailProps) {
  // `data-plan-focus` marks each state's primary control, where focus goes after a sign-in or sign-out.
  const retry = (
    <SettingsButton data-plan-focus="" disabled={busy} onClick={onRetry}>
      {CHATGPT_PLAN_COPY.tryAgain}
    </SettingsButton>
  );

  if (status === null) {
    return busy ? <LoadingLine text={CHATGPT_PLAN_COPY.loading} /> : <StateLine action={retry} />;
  }

  const signInButton = (
    <SettingsButton data-plan-focus="" disabled={busy} variant="primary" onClick={onSignIn}>
      {CHATGPT_PLAN_COPY.signIn}
    </SettingsButton>
  );

  // The sign-in call resolves only when the browser step ends, so the waiting
  // state comes from this hook's own sign-in as well as from the status.
  const state: ChatgptPlanState = signingIn ? "signing_in" : status.state;
  switch (state) {
    case "signing_in":
      return (
        <StateLine action={<SettingsButton onClick={onCancelSignIn}>{CHATGPT_PLAN_COPY.cancelSignIn}</SettingsButton>}>
          {CHATGPT_PLAN_COPY.signingIn}
        </StateLine>
      );
    case "signed_out":
      return <StateLine action={signInButton}>{CHATGPT_PLAN_COPY.signedOut}</StateLine>;
    case "plan_usage_disabled":
      return <StateLine action={signInButton}>{CHATGPT_PLAN_COPY.planUsageDisabled}</StateLine>;
    case "needs_sign_in":
      return <StateLine action={signInButton}>{CHATGPT_PLAN_COPY.needsSignIn}</StateLine>;
    case "check_failed":
      // The reason, when there is one, is the card's alert. The generic line stands in only without a reason.
      return <StateLine action={retry}>{status.error ? null : CHATGPT_PLAN_COPY.checkFailed}</StateLine>;
    case "ready":
      return (
        <div className="flex flex-col items-start gap-3 type-caption text-secondary">
          <p className="type-body text-foreground">{CHATGPT_PLAN_COPY.signedInAs(status.account_label)}</p>
          <div className="flex flex-wrap items-center gap-2">
            <Badge tone="accent">{CHATGPT_PLAN_COPY.usingPlan}</Badge>
            <span>{CHATGPT_PLAN_COPY.modelCount(status.model_count)}</span>
          </div>
          <div className="flex flex-wrap items-center gap-3">
            <Link href={status.usage_url} showExternalIcon>
              {CHATGPT_PLAN_COPY.manageUsage}
            </Link>
            <SettingsButton data-plan-focus="" disabled={busy} onClick={onSignOut}>
              {CHATGPT_PLAN_COPY.signOut}
            </SettingsButton>
          </div>
        </div>
      );
  }
}

function StateLine({ children, action }: { children?: ReactNode; action?: ReactNode }) {
  return (
    <div className="flex flex-col items-start gap-2 type-caption text-secondary">
      {children}
      {action}
    </div>
  );
}

function LoadingLine({ text }: { text: string }) {
  return (
    <div className="flex items-center gap-2 type-caption text-secondary" role="status">
      <Spinner decorative size={16} />
      <span>{text}</span>
    </div>
  );
}
