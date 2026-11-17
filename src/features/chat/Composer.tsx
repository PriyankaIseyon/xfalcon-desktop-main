import {
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useCallback,
  useRef,
  useState,
  type FormEvent,
  type ClipboardEvent,
  type KeyboardEvent,
} from "react";
import { ArrowUp, ChevronDown, FileSpreadsheet, FileText, Folder, Plus, RotateCcw, Sparkles, X } from "lucide-react";
import { IconButton } from "../../ui/IconButton";
import { Textarea } from "../../ui/Field";
import { Tooltip } from "../../ui/Tooltip";
import { Menu } from "../../ui/Menu";
import { Button } from "../../ui/Button";
import { TruncatedText } from "../../ui/TruncatedText";
import { cn } from "../../lib/cn";
import { ComposerPalette } from "./ComposerPalette";
import { readClipboardText } from "./clipboard";
import { clipboardFiles } from "./clipboardAttachments";
import { DEFAULT_SLASH_COMMANDS, extractSlashCommand, type PaletteItem, type PaletteKind } from "./composerDefaults";
import type { UploadProjectionV1 } from "../../lib/generated/contracts";

/** Autosize cap (spec: chat-composer): the input grows with the draft up to
 * twelve lines, then scrolls internally. Raised 10% with the resting height
 * below, so a taller composer does not start scrolling a line sooner. */
const COMPOSER_MAX_HEIGHT_PX = 330;

const TRANSFERRING_STATES = new Set(["selected", "preparing", "uploading", "resigning", "finalizing"]);

function attachmentProgress(upload: UploadProjectionV1): number {
  if (upload.size === 0) return upload.state === "ready" ? 1 : 0;
  return Math.min(1, upload.bytes_sent / upload.size);
}

export interface ComposerProps {
  /** The secure runtime is unavailable — typing and sending are both off.
   * A merely in-flight run is NOT this; pass `streaming` for that. */
  disabled?: boolean;
  onSubmit?: (prompt: string, attachmentIds?: string[]) => void;
  placeholder?: string;
  draft?: string;
  onDraftChange?: (draft: string) => void;
  /** A run is in flight: the circular control becomes Stop, submission is
   * gated, typing stays live (spec: chat-composer). */
  streaming?: boolean;
  /** A request is being admitted but has no run to stop yet. Submission and
   * attachment actions wait; typing stays live. */
  submitting?: boolean;
  /** Cancels the active run. Only reachable while `streaming`. */
  onStop?: () => void;
  /** Stop has been pressed and the cancel has not come back yet. */
  stopping?: boolean;
  /** Display name of the trusted project a submitted message would run
   * against, or `null`/omitted when no project is active. Purely a compact,
   * quiet label above the input -- it never gates `disabled`/`onSubmit` and
   * carries no trust or approval semantics of its own (those live in
   * `ProjectCreateSheet`/`ApprovalSheet`). */
  activeProjectName?: string | null;
  /** Starts a fresh ordinary chat without changing the saved project chat. */
  onClearProject?: () => void;
  /** Optional project file names for @ mentions */
  projectFiles?: string[];
  /** Optional configured MCP server names for @ mentions */
  mcpServers?: string[];
  /** Imported skills the composer can @mention or attach via /skill */
  skills?: Array<{ name: string; description: string }>;
  /** Handler for executing slash commands */
  onSlashCommand?: (command: string) => void;
  /** Opens the native file picker. Live whenever the composer can send. */
  onAttach?: () => void;
  /** Empty files asks Rust to read native clipboard file references. */
  onPasteFiles?: (files: File[]) => Promise<void>;
  /** Stable admission state survives switching away from the chat view. */
  attachmentPending?: boolean;
  /** The immutable model for this chat, or the default for a new chat. */
  modelName?: string;
  /** Says what the model runs on, beside its name (for example the plan it uses). */
  modelNote?: string;
  mcpSelection?: { serverId: string | null; label: string; loading?: boolean; error?: string | null };
  mcpServerOptions?: Array<{ id: string; label: string }>;
  mcpSelectionDisabled?: boolean;
  onMcpServerSelect?: (serverId: string | null) => void;
  /** Composer uploads for the open conversation. */
  attachments?: UploadProjectionV1[];
  onCancelUpload?: (uploadId: string) => void;
  onRetryUpload?: (uploadId: string) => void;
}

/**
 * Minimal composer card (spec: `chat-composer`): a single soft-rounded
 * surface — borderless input on top, then one action row with the attach
 * affordance and immutable model name at the left and a circular send/stop at
 * the right. The model name is status text, not a switch.
 */
export function Composer({
  disabled = false,
  onSubmit,
  placeholder = "Ask about data",
  draft,
  onDraftChange,
  streaming = false,
  submitting = false,
  onStop,
  stopping = false,
  activeProjectName,
  onClearProject,
  projectFiles = [],
  mcpServers = [],
  skills = [],
  onSlashCommand,
  onAttach,
  onPasteFiles,
  attachmentPending = false,
  modelName,
  modelNote,
  mcpSelection,
  mcpServerOptions = [],
  mcpSelectionDisabled = false,
  onMcpServerSelect,
  attachments = [],
  onCancelUpload,
  onRetryUpload,
}: ComposerProps) {
  const [pastingFiles, setPastingFiles] = useState(false);
  const pasteInFlight = useRef(false);
  const [localPrompt, setLocalPrompt] = useState("");
  const controlled = draft !== undefined && onDraftChange !== undefined;
  const prompt = controlled ? draft : localPrompt;
  const setPrompt = controlled ? onDraftChange : setLocalPrompt;
  const hintId = useId();
  const available = !disabled && onSubmit !== undefined;
  const transferring = attachments.some((upload) => TRANSFERRING_STATES.has(upload.state));
  const readyIds = attachments
    .filter((upload) => upload.state === "ready" && upload.resource_id)
    .map((upload) => upload.resource_id as string);
  const canSubmit =
    available &&
    !streaming &&
    !submitting &&
    !pastingFiles &&
    !attachmentPending &&
    !transferring &&
    (prompt.trim().length > 0 || readyIds.length > 0);
  const showStop = streaming && onStop !== undefined;
  const attachLive =
    available && !streaming && !submitting && !pastingFiles && !attachmentPending && onAttach !== undefined;

  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const composingRef = useRef(false);

  // Palette State (@ mentions and / slash commands)
  const [paletteKind, setPaletteKind] = useState<PaletteKind | null>(null);
  const [paletteQuery, setPaletteQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);

  const [skillOnly, setSkillOnly] = useState(false);

  const closePalette = useCallback(() => {
    setPaletteKind(null);
    setPaletteQuery("");
    setSkillOnly(false);
  }, []);

  const fileMentionItems = useMemo<PaletteItem[]>(() => {
    return projectFiles.map((filename) => ({
      id: `file-${filename}`,
      kind: "mention" as const,
      category: "file" as const,
      title: `@${filename}`,
      subtitle: "Project File",
      insertText: `@${filename} `,
      icon:
        filename.endsWith(".csv") || filename.endsWith(".xlsx") ? (
          <FileSpreadsheet size={15} />
        ) : (
          <FileText size={15} />
        ),
    }));
  }, [projectFiles]);

  const mcpMentionItems = useMemo<PaletteItem[]>(() => {
    return mcpServers.map((serverName) => ({
      id: `mcp-${serverName}`,
      kind: "mention" as const,
      category: "mcp" as const,
      title: `@mcp:${serverName}`,
      subtitle: "MCP Server",
      insertText: `@mcp:${serverName} `,
      icon: <Plus size={15} />,
    }));
  }, [mcpServers]);

  const skillMentionItems = useMemo<PaletteItem[]>(() => {
    return skills.map((skill) => ({
      id: `skill-${skill.name}`,
      kind: "mention" as const,
      category: "skill" as const,
      title: `@skill:${skill.name}`,
      subtitle: skill.description,
      insertText: `@skill:${skill.name} `,
      icon: <Sparkles size={15} />,
    }));
  }, [skills]);

  const allMentionItems = useMemo(() => {
    return [...fileMentionItems, ...mcpMentionItems, ...skillMentionItems];
  }, [fileMentionItems, mcpMentionItems, skillMentionItems]);

  const paletteItems = useMemo(() => {
    if (!paletteKind) return [];
    const query = paletteQuery.toLowerCase().trim();
    if (paletteKind === "slash") {
      return DEFAULT_SLASH_COMMANDS.filter(
        (cmd) =>
          cmd.title.toLowerCase().includes(query) || (cmd.subtitle && cmd.subtitle.toLowerCase().includes(query)),
      );
    }
    return allMentionItems.filter((item) => {
      if (skillOnly && item.category !== "skill") return false;
      return item.title.toLowerCase().includes(query) || (item.subtitle && item.subtitle.toLowerCase().includes(query));
    });
  }, [paletteKind, paletteQuery, allMentionItems, skillOnly]);

  useEffect(() => {
    setSelectedIndex(0);
  }, [paletteItems.length, paletteKind, paletteQuery]);

  /**
   * `requestAnimationFrame` used to carry this. It waits for the next paint,
   * which is later than the next keystroke can arrive: `userEvent.type` (and
   * a fast typist) can fire several keydowns inside that one frame, so the
   * frame's `setSelectionRange` landed mid-sequence and scrambled whatever
   * had already been typed after it -- "next" arriving as "extn", caught by
   * a 2-in-5 flake on the regression test this exact call exists to satisfy.
   * A layout effect keyed on `prompt` runs synchronously right after React
   * commits the new value (so the palette is already gone from the DOM,
   * which is what the original deferral was protecting) and strictly before
   * the browser can process another input event -- no window for a keystroke
   * to land in between.
   */
  const pendingCaretRef = useRef<number | null>(null);
  const focusTextareaAt = (position: number) => {
    pendingCaretRef.current = position;
  };
  useLayoutEffect(() => {
    const position = pendingCaretRef.current;
    if (position === null) return;
    pendingCaretRef.current = null;
    const textarea = textareaRef.current;
    if (!textarea) return;
    textarea.focus();
    textarea.setSelectionRange(position, position);
  }, [prompt]);

  const checkTriggers = (value: string, caret?: number) => {
    const cursor = caret ?? textareaRef.current?.selectionStart ?? value.length;
    const textBeforeCursor = value.slice(0, cursor);

    // Check for slash command at the beginning of prompt only
    const slashMatch = textBeforeCursor.match(/^\/([a-zA-Z0-9_-]*)$/);
    if (slashMatch) {
      setPaletteKind("slash");
      setPaletteQuery(slashMatch[1]);
      setSkillOnly(false);
      return;
    }

    // Check for @ mention trigger (must be preceded by start-of-line or whitespace to avoid emails)
    const mentionMatch = textBeforeCursor.match(/(?:^|\s)@([a-zA-Z0-9_.:-]*)$/);
    if (mentionMatch) {
      setPaletteKind("mention");
      setPaletteQuery(mentionMatch[1]);
      setSkillOnly(mentionMatch[1].toLowerCase().startsWith("skill"));
      return;
    }

    setPaletteKind(null);
    setPaletteQuery("");
    setSkillOnly(false);
  };

  const handlePromptChange = (val: string) => {
    setPrompt(val);
    checkTriggers(val);
  };

  /**
   * WebView clipboard behavior differs by platform when a controlled textarea
   * receives rich clipboard data. Own the plain-text insertion so a paste has
   * one contract everywhere: replace the current selection, preserve line
   * breaks, and leave the caret after the inserted text. Files remain on the
   * attachment path rather than being coerced into a prompt string.
   */
  const handlePaste = (event: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = clipboardFiles(event.clipboardData);
    const pasted = files.length ? "" : readClipboardText(event.clipboardData);
    const nativeFiles =
      Array.from(event.clipboardData.types ?? []).includes("Files") ||
      event.clipboardData.getData("text/uri-list").trimStart().startsWith("file:") ||
      (!pasted && !event.clipboardData.getData("text/html"));
    if (files.length || (nativeFiles && onPasteFiles)) {
      event.preventDefault();
      if (!available || streaming || submitting || attachmentPending || pasteInFlight.current || !onPasteFiles) return;
      pasteInFlight.current = true;
      setPastingFiles(true);
      void onPasteFiles(files).finally(() => {
        pasteInFlight.current = false;
        setPastingFiles(false);
      });
      return;
    }
    if (!pasted) return;

    event.preventDefault();
    const start = event.currentTarget.selectionStart ?? prompt.length;
    const end = event.currentTarget.selectionEnd ?? start;
    const text = pasted.replace(/\r\n?/g, "\n");
    const nextPrompt = `${prompt.slice(0, start)}${text}${prompt.slice(end)}`;
    const nextCaret = start + text.length;

    setPrompt(nextPrompt);
    checkTriggers(nextPrompt, nextCaret);
    focusTextareaAt(nextCaret);
  };

  const resizeToContent = () => {
    const el = textareaRef.current;
    if (!el) return;
    // Measure from a collapsed box, then cap. Scrolling stays OFF below the
    // cap: `scrollHeight` is an integer while the reading type is a fluid
    // `clamp()`, so a standing `overflow-y: auto` shows a scrollbar on a
    // composer with nothing in it. It turns on at the exact line the draft
    // outgrows the cap and off again when the draft shrinks back.
    el.style.height = "auto";
    const content = el.scrollHeight;
    el.style.height = `${Math.min(content, COMPOSER_MAX_HEIGHT_PX)}px`;
    el.style.overflowY = content > COMPOSER_MAX_HEIGHT_PX ? "auto" : "hidden";
  };

  // Grow with the draft, shrink after send (spec: chat-composer). Skipped
  // mid-IME-composition — resizing under a candidate window fights the IME;
  // compositionend runs one settling pass instead.
  useLayoutEffect(() => {
    if (composingRef.current) return;
    resizeToContent();
  }, [prompt]);

  // The caret rests here on arrival: the composer is what the chat surface is
  // for, and typing should not cost a click. Once only, and only when the
  // runtime can actually accept a message — focusing a dead input would put
  // the caret somewhere that ignores the keyboard. Later re-renders never
  // re-take focus, so this cannot pull it back out of a dialog or the
  // transcript.
  const focusedOnceRef = useRef(false);
  useEffect(() => {
    if (focusedOnceRef.current || !available) return;
    focusedOnceRef.current = true;
    textareaRef.current?.focus();
  }, [available]);

  // The composer owns keyboard focus at both ends of a turn. Pressing Enter
  // already leaves focus here; tool approvals and transcript interactions can
  // move it while the run works, so transfer it back on the single busy -> idle
  // edge. A layout effect observes the committed control state, and the
  // microtask lets any completing button event finish before focus moves. Do
  // not steal focus from another editable control or an open modal.
  const wasBusyRef = useRef(streaming || submitting);
  useLayoutEffect(() => {
    const busy = streaming || submitting;
    const completed = wasBusyRef.current && !busy;
    wasBusyRef.current = busy;
    if (!completed || !available) return;

    queueMicrotask(() => {
      const textarea = textareaRef.current;
      if (!textarea || textarea.disabled || !textarea.isConnected) return;
      const active = document.activeElement;
      const activeIsEditable =
        active instanceof HTMLElement &&
        (active.isContentEditable || active.matches("input, textarea, select, [role='textbox']"));
      if (active !== textarea && (activeIsEditable || document.querySelector("[role='dialog'][aria-modal='true']")))
        return;
      textarea.focus({ preventScroll: true });
      textarea.setSelectionRange(textarea.value.length, textarea.value.length);
    });
  }, [available, streaming, submitting]);

  const handleSelectPaletteItem = (item: PaletteItem) => {
    if (item.onSelect) {
      item.onSelect();
    }
    if (item.kind === "slash") {
      const match = extractSlashCommand(item.insertText);
      if (match === "skill" && skills.length > 0) {
        setPrompt("");
        setPaletteKind("mention");
        setPaletteQuery("");
        setSkillOnly(true);
        focusTextareaAt(0);
        return;
      }
      if (match && onSlashCommand) {
        onSlashCommand(match);
        setPrompt("");
        closePalette();
        return;
      }
    }

    const el = textareaRef.current;
    const cursor = el ? el.selectionStart : prompt.length;
    const textBeforeCursor = prompt.slice(0, cursor);
    const textAfterCursor = prompt.slice(cursor);

    let newPrompt: string;
    let nextCaret: number;
    if (item.kind === "slash") {
      newPrompt = item.insertText + textAfterCursor;
      nextCaret = item.insertText.length;
    } else {
      const atPos = textBeforeCursor.lastIndexOf("@");
      if (atPos !== -1) {
        newPrompt = textBeforeCursor.slice(0, atPos) + item.insertText + textAfterCursor;
        nextCaret = textBeforeCursor.slice(0, atPos).length + item.insertText.length;
      } else if (!prompt.trim()) {
        newPrompt = item.insertText;
        nextCaret = item.insertText.length;
      } else {
        newPrompt = `${prompt} ${item.insertText}`;
        nextCaret = newPrompt.length;
      }
    }

    setPrompt(newPrompt);
    closePalette();
    focusTextareaAt(nextCaret);
  };

  const submitPrompt = () => {
    const value = prompt.trim();
    if (!available || streaming || submitting || transferring || attachmentPending || pasteInFlight.current) return;
    if (!value && readyIds.length === 0) return;

    const match = extractSlashCommand(value);
    if (match === "skill" && skills.length > 0) {
      setPrompt("");
      setPaletteKind("mention");
      setPaletteQuery("");
      setSkillOnly(true);
      return;
    }
    if (match && onSlashCommand) {
      onSlashCommand(match);
      setPrompt("");
      closePalette();
      return;
    }

    if (readyIds.length > 0) {
      onSubmit?.(value, readyIds);
    } else {
      onSubmit?.(value);
    }
    setPrompt("");
    closePalette();
  };

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    submitPrompt();
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (paletteKind && event.key === "Escape") {
      event.preventDefault();
      closePalette();
      return;
    }

    if (paletteKind && paletteItems.length > 0) {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setSelectedIndex((prev) => (prev + 1) % paletteItems.length);
        return;
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        setSelectedIndex((prev) => (prev - 1 + paletteItems.length) % paletteItems.length);
        return;
      }
      if (event.key === "Enter" || event.key === "Tab") {
        event.preventDefault();
        const selected = paletteItems[selectedIndex];
        if (selected) {
          handleSelectPaletteItem(selected);
        }
        return;
      }
    }

    if (event.key !== "Enter" || event.shiftKey) return;
    // Enter is the candidate-commit key for CJK input methods; a composing
    // Enter must never send the message.
    if (event.nativeEvent.isComposing) return;
    event.preventDefault();
    submitPrompt();
  };

  return (
    <form aria-label="Message composer" className="composer relative w-full" onSubmit={submit}>
      {paletteKind ? (
        <ComposerPalette
          items={paletteItems}
          kind={paletteKind}
          onClose={closePalette}
          onSelect={handleSelectPaletteItem}
          query={paletteQuery}
          selectedIndex={selectedIndex}
        />
      ) : null}

      {activeProjectName ? (
        <div className="composer-project-context">
          <Folder aria-hidden size={12} strokeWidth={1.75} />
          <span>{activeProjectName}</span>
          {onClearProject ? (
            <Tooltip label="Use a normal chat">
              <button
                aria-label={`Remove ${activeProjectName} project context`}
                className="chip-action"
                type="button"
                onClick={onClearProject}
              >
                <X aria-hidden size={12} strokeWidth={2} />
              </button>
            </Tooltip>
          ) : null}
        </div>
      ) : null}
      <label className="sr-only" htmlFor={hintId}>
        Message xFalcon
      </label>
      <Textarea
        id={hintId}
        ref={textareaRef}
        aria-describedby={`${hintId}-hint`}
        aria-expanded={Boolean(paletteKind)}
        aria-haspopup="listbox"
        aria-controls={paletteKind ? "composer-palette-list" : undefined}
        aria-activedescendant={
          paletteKind && paletteItems[selectedIndex]
            ? `composer-palette-item-${paletteItems[selectedIndex].id}`
            : undefined
        }
        className={cn(
          // The explicit 24px line box is load-bearing: the reading size is a
          // fluid `clamp()`, so a ratio line-height lands on a fraction and
          // the measured integer height falls a hair short of the content.
          // Height only: `min-h` and the cap carry the 10% increase, and the
          // horizontal padding is deliberately untouched.
          "max-h-[330px] min-h-[27px] overflow-hidden border-0 bg-transparent px-1 py-1 text-reading leading-[24px] shadow-none",
          "focus:border-transparent focus:ring-0",
        )}
        disabled={!available}
        onChange={(event) => handlePromptChange(event.currentTarget.value)}
        onCompositionEnd={() => {
          composingRef.current = false;
          resizeToContent();
        }}
        onCompositionStart={() => {
          composingRef.current = true;
        }}
        onKeyDown={handleKeyDown}
        onPaste={handlePaste}
        placeholder={`\u00a0\u00a0${available ? placeholder : "Chat is not connected yet"}`}
        rows={1}
        value={prompt}
      />
      <span className="sr-only" id={`${hintId}-hint`}>
        {available
          ? "Press Enter to send, Shift+Enter for a new line, @ for context, / for commands."
          : "Chat becomes available after the secure runtime connects."}
      </span>
      {attachments.length > 0 ? (
        <ul className="flex flex-wrap gap-1.5 px-1">
          {attachments
            .filter((upload) => upload.state !== "cancelled")
            .map((upload) => {
              const failed = upload.state === "failed";
              const progress = Math.round(attachmentProgress(upload) * 100);
              return (
                <li key={upload.id} className="max-w-[14rem]">
                  <span
                    className={cn(
                      "inline-flex max-w-[14rem] items-center gap-1 rounded-pill border px-2 py-1 text-xs",
                      failed ? "border-danger/40 text-danger" : "border-default text-secondary",
                    )}
                  >
                    <TruncatedText className="min-w-0">{upload.display_name}</TruncatedText>
                    {TRANSFERRING_STATES.has(upload.state) ? (
                      <span className="tabular-nums text-faint">{progress}%</span>
                    ) : null}
                    {failed && onRetryUpload ? (
                      <button
                        aria-label={`Retry ${upload.display_name}`}
                        className="press chip-action"
                        type="button"
                        onClick={() => onRetryUpload(upload.id)}
                      >
                        <RotateCcw aria-hidden size={12} />
                      </button>
                    ) : null}
                    {onCancelUpload ? (
                      <button
                        aria-label={`Remove ${upload.display_name}`}
                        className="press chip-action"
                        type="button"
                        onClick={() => onCancelUpload(upload.id)}
                      >
                        <X aria-hidden size={12} />
                      </button>
                    ) : null}
                  </span>
                  {failed ? (
                    <p className="mt-1 text-xs leading-snug text-danger" role="status">
                      Upload failed: {upload.safe_error ?? "The upload stopped before it finished."}
                    </p>
                  ) : null}
                </li>
              );
            })}
        </ul>
      ) : null}
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <IconButton
            aria-label="Attach files"
            className="text-faint"
            disabled={!attachLive}
            size="sm"
            type="button"
            onClick={onAttach}
          >
            <Plus aria-hidden size={16} strokeWidth={1.8} />
          </IconButton>
          {modelName ? (
            <Tooltip label="Change models in Settings. A chat keeps the model it starts with.">
              <span aria-label={`Model: ${modelName}`} className="composer-model-name" tabIndex={0}>
                {modelName}
              </span>
            </Tooltip>
          ) : null}
          {modelNote ? <span className="type-caption shrink-0 whitespace-nowrap text-faint">{modelNote}</span> : null}
          {mcpSelection && onMcpServerSelect ? (
            <Menu
              trigger={
                <Button
                  aria-label="MCP server"
                  className="max-w-40 gap-1 truncate"
                  disabled={mcpSelectionDisabled || mcpSelection.loading || attachmentPending || transferring}
                  size="sm"
                  variant="ghost"
                >
                  <span className="truncate">{mcpSelection.label}</span>
                  <ChevronDown aria-hidden size={13} />
                </Button>
              }
              align="start"
              className="w-56"
            >
              <Menu.Label>MCP server</Menu.Label>
              <Menu.Item selected={mcpSelection.serverId === null} onSelect={() => onMcpServerSelect(null)}>
                All servers
              </Menu.Item>
              {mcpSelection.serverId !== null &&
              !mcpServerOptions.some((server) => server.id === mcpSelection.serverId) ? (
                <Menu.Item disabled selected>
                  {mcpSelection.label} (unavailable)
                </Menu.Item>
              ) : null}
              {mcpServerOptions.map((server) => (
                <Menu.Item
                  key={server.id}
                  selected={mcpSelection.serverId === server.id}
                  onSelect={() => onMcpServerSelect(server.id)}
                >
                  {server.label}
                </Menu.Item>
              ))}
            </Menu>
          ) : null}
        </div>
        {showStop ? (
          <IconButton
            aria-label={stopping ? "Stopping the assistant's response" : "Stop the assistant's response"}
            className="composer-stop"
            loading={stopping}
            shape="circle"
            size="md"
            type="button"
            onClick={onStop}
          >
            <span aria-hidden className="composer-stop__glyph" />
          </IconButton>
        ) : (
          <IconButton
            aria-label={submitting ? "Chat request in progress" : "Send message"}
            loading={submitting}
            shape="circle"
            variant="primary"
            disabled={!canSubmit}
            size="md"
            type="submit"
          >
            <ArrowUp aria-hidden size={16} strokeWidth={1.8} />
          </IconButton>
        )}
      </div>
    </form>
  );
}
