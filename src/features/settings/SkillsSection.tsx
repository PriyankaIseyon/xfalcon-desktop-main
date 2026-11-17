import React from "react";
import { Plus } from "lucide-react";
import type { SkillView } from "../../lib/generated/contracts";
import { skillDelete, skillList, skillSetEnabled } from "../../lib/native";
import { Alert } from "../../ui/Alert";
import { AlertDialog } from "../../ui/AlertDialog";
import { EmptyState } from "../../ui/EmptyState";
import { List, ListItem } from "../../ui/List";
import { Spinner } from "../../ui/Spinner";
import { Switch } from "../../ui/Switch";
import { toast } from "../../ui/toastStore";
import { SkillImportDialog } from "./SkillImportDialog";
import { cn } from "../../lib/cn";
import { SettingsButton, SettingsRemoveButton } from "./SettingsControls";
import { SettingsGroup, SettingsGroupHeader } from "./SettingsLayout";
import { SKILLS_COPY, describeSkillError } from "./settingsCopy";
import { settingsItemElementId } from "../../app/settingsCatalog";
import { HighlightedText, SearchRevealHost } from "../../lib/highlightedText";

type ListState =
  { status: "loading" } | { status: "ready"; skills: SkillView[] } | { status: "error"; message: string };

/**
 * The imported skill library (spec: user-skills). A block inside
 * AgentCapabilitiesSection: it draws no box of its own.
 *
 * This surface lists and governs what is already imported; getting skills in is
 * [`SkillImportDialog`]'s job. One button opens it, rather than a button per
 * input kind here: a file, an archive and a folder are three spellings of the
 * same intent, and offering them as separate controls asked the reader to pick
 * between them before anything had explained the difference.
 */
export function SkillsSection() {
  const [state, setState] = React.useState<ListState>({ status: "loading" });
  const [reloadToken, setReloadToken] = React.useState(0);
  const [importOpen, setImportOpen] = React.useState(false);
  const [pending, setPending] = React.useState<string | null>(null);
  const [pendingRemoval, setPendingRemoval] = React.useState<SkillView | null>(null);
  const [removeError, setRemoveError] = React.useState<string | null>(null);

  React.useEffect(() => {
    let active = true;
    skillList()
      .then((result) => {
        if (active) setState({ status: "ready", skills: result.skills });
      })
      .catch((cause: unknown) => {
        if (active) setState({ status: "error", message: describeSkillError(cause, SKILLS_COPY.listFailed) });
      });
    return () => {
      active = false;
    };
  }, [reloadToken]);

  const reload = React.useCallback(() => {
    setState({ status: "loading" });
    setReloadToken((token) => token + 1);
  }, []);

  const toggle = async (skill: SkillView) => {
    if (pending !== null) return;
    setPending(skill.name);
    try {
      const updated = await skillSetEnabled(skill.name, !skill.enabled);
      setState((current) =>
        current.status === "ready"
          ? {
              status: "ready",
              skills: current.skills.map((row) => (row.name === updated.name ? updated : row)),
            }
          : current,
      );
    } catch (cause: unknown) {
      toast.error(describeSkillError(cause, SKILLS_COPY.toggleFailed));
    } finally {
      setPending(null);
    }
  };

  const remove = async (skill: SkillView) => {
    if (pending !== null) return;
    setPending(skill.name);
    try {
      await skillDelete(skill.name);
      toast.success(`Removed “${skill.name}”.`);
      setPendingRemoval(null);
      reload();
    } catch (cause: unknown) {
      setRemoveError(describeSkillError(cause, SKILLS_COPY.deleteFailed));
    } finally {
      setPending(null);
    }
  };

  return (
    <SettingsGroup id="settings-skills" aria-labelledby="settings-skills-heading">
      <SettingsGroupHeader
        headingId="settings-skills-heading"
        title={SKILLS_COPY.title}
        description={SKILLS_COPY.description}
      >
        <SettingsButton icon={<Plus />} onClick={() => setImportOpen(true)}>
          {SKILLS_COPY.importLabel}
        </SettingsButton>
      </SettingsGroupHeader>

      {state.status === "loading" ? (
        <div className="settings-group__state text-secondary">
          <Spinner decorative size={16} />
          <span>Loading skills…</span>
        </div>
      ) : null}

      {state.status === "error" ? (
        <Alert className="settings-group__state" title="Skills unavailable" tone="danger">
          <p>{state.message}</p>
          <SettingsButton className="mt-2" onClick={reload}>
            Try again
          </SettingsButton>
        </Alert>
      ) : null}

      {state.status === "ready" && state.skills.length > 0 ? (
        <List className="flex flex-col gap-0">
          {state.skills.map((skill) => (
            <ListItem key={skill.name}>
              <SearchRevealHost id={settingsItemElementId("skill", skill.name)}>
                <div className="settings-collection-item settings-collection-item--skill">
                  <div className="settings-skill__copy min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <p className="type-label break-all font-semibold text-foreground">
                        <HighlightedText text={skill.name} />
                      </p>
                      <span
                        className={cn(
                          "settings-skill__status",
                          skill.enabled ? "settings-skill__status--on" : "settings-skill__status--off",
                        )}
                      >
                        {skill.enabled ? "Active" : "Off"}
                      </span>
                    </div>
                    <p className="type-caption mt-1 break-words text-secondary">
                      <HighlightedText text={skill.description} />
                    </p>
                    <p className="type-caption mt-1 text-muted">
                      {skill.enabled ? SKILLS_COPY.enabledHint : SKILLS_COPY.disabledHint}
                      {skill.bundled_files > 0 ? ` · ${SKILLS_COPY.bundledFiles(skill.bundled_files)}` : null}
                    </p>
                  </div>
                  <div className="settings-actions settings-skill__actions shrink-0">
                    <Switch
                      checked={skill.enabled}
                      disabled={pending === skill.name}
                      label={<span className="sr-only">{`Use ${skill.name}`}</span>}
                      onChange={() => void toggle(skill)}
                    />
                    <SettingsRemoveButton
                      aria-label={`Remove ${skill.name}`}
                      disabled={pending === skill.name}
                      onClick={() => {
                        setRemoveError(null);
                        setPendingRemoval(skill);
                      }}
                    />
                  </div>
                </div>
              </SearchRevealHost>
            </ListItem>
          ))}
        </List>
      ) : null}

      {/* No action here: the header's button is the one way in, and repeating
          it beside this text is what made four controls out of two. */}
      {state.status === "ready" && state.skills.length === 0 ? (
        <EmptyState
          className="settings-group__empty"
          description={SKILLS_COPY.emptyBody}
          title={<p className="type-body font-semibold text-foreground">{SKILLS_COPY.emptyTitle}</p>}
        />
      ) : null}

      <SkillImportDialog open={importOpen} onClose={() => setImportOpen(false)} onImported={reload} />
      <AlertDialog
        busy={pending === pendingRemoval?.name}
        confirmLabel="Remove skill"
        description={
          pendingRemoval
            ? `“${pendingRemoval.name}” and its imported files will be removed from this device. Import the skill again to restore it.`
            : ""
        }
        destructive
        error={removeError}
        open={pendingRemoval !== null}
        onCancel={() => {
          setPendingRemoval(null);
          setRemoveError(null);
        }}
        onConfirm={() => {
          if (pendingRemoval) void remove(pendingRemoval);
        }}
        title={pendingRemoval ? `Remove “${pendingRemoval.name}”?` : "Remove skill?"}
      />
    </SettingsGroup>
  );
}
