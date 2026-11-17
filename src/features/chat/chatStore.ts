import { useSyncExternalStore } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  ChatConversationSummary,
  ChatConversationView,
  ChatProjectionEvent,
  ChatSyncAvailability,
  ChatAttachmentView,
  ChatToolCallView,
  InferenceEvent,
  RunEvent,
  ToolInvocationEvent,
} from "../../lib/generated/contracts";
import {
  CHAT_INFERENCE_EVENT_CHANNEL,
  CHAT_PROJECTION_EVENT_CHANNEL,
  CHAT_RUN_EVENT_CHANNEL,
  TOOL_INVOCATION_EVENT_CHANNEL,
} from "./chatChannels";
import {
  captureInferenceSequenceState,
  forgetInferenceSequenceState,
  reduceInferenceEvent,
  reduceRunEvent,
  resetInferenceSequenceState,
  restoreInferenceSequenceState,
  toolRowCount,
  type InferenceSequenceSnapshot,
} from "./chatReduce";
import { resetComposerDraft } from "./composerStore";
import { resetUploadQueue } from "../uploads/uploadQueueStore";
import { clearTurnEdit } from "./turnEditStore";
import { getAuthAccountGeneration, getAuthSession } from "../auth/authStore";
import { resetTurnClock, syncTurnClock } from "./runClock";
import { isBoundRunPending } from "./runState";

export type ChatMessageRow = {
  id: string;
  role: "user" | "assistant" | "system";
  text: string;
  sequence: bigint;
  /**
   * The tool round this turn performed, from the durable conversation. Empty
   * for a turn that ran none, and the only trace of a round that survives a
   * reload -- `activity` below is live, per-run state that the backend never
   * replays.
   */
  toolCalls: ChatToolCallView[];
  /** The files sent with this turn. Absent on a turn that carried none. */
  attachments?: ChatAttachmentView[];
};

export type ChatStoreSnapshot = {
  conversationId: string | null;
  /**
   * A conversation made to hold files attached on the new-chat screen, before
   * anything was sent. DA takes an attachment only into a conversation, but
   * the person has not started one, so this one is not opened: it becomes the
   * open conversation when its first message is sent, and is deleted if they
   * go elsewhere instead (spec: chat-attachments). `null` whenever a
   * conversation is open.
   */
  reservedConversationId: string | null;
  title: string;
  availability: ChatSyncAvailability | null;
  messages: ChatMessageRow[];
  /** Text of a run that is still producing tokens. Empty the moment its
   * terminal inference event lands — from then on the same text lives in
   * `settlingText` until the durable row replaces it. */
  streamingText: string;
  /**
   * The run's streamed answer after its terminal inference event and before
   * its durable row arrives.
   *
   * That gap is real — `finish_run` reconciles after the terminal event,
   * with retries — and without holding the text across it the answer blanks
   * and repaints. It retires on exactly the signal that retires `activity`
   * and `roundProse`, so the live surfaces and the durable row can never
   * both be showing the same answer (spec: chat-run-activity).
   */
  settlingText: string;
  /**
   * The run that is shortening its history to fit the model's window, while it
   * does. A run id rather than a flag, so a value left behind by a run that
   * ended or a conversation that was left can never label a different run: it
   * counts only while it equals `activeRunId`.
   */
  compactingRunId: string | null;
  activeRunId: string | null;
  runState: string | null;
  error: string | null;
  conversations: ChatConversationSummary[];
  workspaceLoaded: boolean;
  /** The conversation whose authoritative open is still in flight. */
  openingConversationId: string | null;
  /** True only until either a local preview or authoritative view is visible. */
  conversationContentPending: boolean;
  recoveryNotice: string | null;
  /** Live tool-invocation lifecycle for the current conversation, oldest
   * first. Emitted by the agent run's invocation journal, so an entry here
   * always corresponds to a journaled step that really happened. */
  activity: ToolInvocationEvent[];
  /** Completed rounds' streamed prose, captured at each round boundary (the
   * round's first `requested` tool event) — the run's activity card renders
   * these INSIDE itself as thinking segments (spec: chat-run-activity).
   * Live, per-run state like `activity`; cleared with it. */
  roundProse: LiveSegment[];
  /**
   * The run's streamed reasoning, split where tool rows interrupted it.
   *
   * Live per-run state the backend never replays — like `activity`, unlike
   * `messages`. Unlike every other live field it is NOT retired by the durable
   * snapshot: the snapshot carries the answer and the tool rounds, so those
   * retire on it, but nothing in it carries reasoning. Clearing it there would
   * delete the only copy that ever existed — so it outlives the live card and
   * is drawn by the durable umbrella instead (spec: chat-run-activity).
   */
  reasoning: LiveSegment[];
  /** Reasoning passed its retention cap and the oldest text was dropped. The
   * panel says so — a wall of thinking that silently starts mid-sentence reads
   * as a broken model rather than a bound in the app. */
  reasoningTrimmed: boolean;
  /** An optimistic turn revision or edit is in flight. */
  revising: boolean;
  /**
   * A turn has been submitted and no run exists for it yet.
   *
   * Submitting reaches the tenant API — a session refresh, the model catalog,
   * the run row — before it can answer with a run id, so `runState` stays null
   * for that whole round trip and the message the reader just sent sat under
   * dead air until the backend spoke. This is the app's own claim that a run is
   * coming, and it drives the thinking indicator alone: real run state replaces
   * it the moment the submit result lands, and nothing here can cancel a run or
   * accept tool events, because there is no run to bind to yet.
   */
  awaitingRun: boolean;
};

/**
 * One block of a run's narration or reasoning, placed among its tool rows.
 *
 * `afterCallCount` is the ONE axis both surfaces order on: the live card counts
 * the rows it has drawn, the durable umbrella counts the calls it has emitted,
 * and a segment lands after that many in either. Sharing the axis is what lets
 * a run read the same way while it streams and after it settles, rather than
 * re-deriving position from state that only one of the two surfaces has.
 */
export type LiveSegment = {
  text: string;
  /** Tool rows that already existed when this segment was captured. */
  afterCallCount: number;
};

/**
 * Cap on retained tool-invocation events.
 *
 * A run is bounded to `MAX_TOOL_CALLS_PER_RUN` (24) calls and each emits at
 * most a handful of lifecycle events, so this holds several full runs of
 * history. It exists so a long-lived window cannot grow this array without
 * limit; `LiveRunActivity` keys by invocation and shows the latest event
 * per invocation, so dropping the oldest entries only ever loses whole
 * finished invocations, never half of a live one.
 */
const MAX_ACTIVITY_EVENTS = 200;

/** Marks a row this device drew before the backend recorded it. */
const OPTIMISTIC_ROW_PREFIX = "local-";

type Listener = () => void;

const initialState: ChatStoreSnapshot = {
  conversationId: null,
  reservedConversationId: null,
  title: "New conversation",
  availability: null,
  messages: [],
  streamingText: "",
  settlingText: "",
  compactingRunId: null,
  activeRunId: null,
  runState: null,
  error: null,
  conversations: [],
  workspaceLoaded: false,
  openingConversationId: null,
  conversationContentPending: false,
  recoveryNotice: null,
  activity: [],
  roundProse: [],
  reasoning: [],
  reasoningTrimmed: false,
  revising: false,
  awaitingRun: false,
};

let state: ChatStoreSnapshot = { ...initialState };
// Successful native deletion is authoritative over older in-flight responses.
// Account reset clears these session-local fences; native storage persists them.
const removedConversations = new Set<string>();

/** The sidebar's rows: never a deleted conversation, and never the one
 * reserved for the new-chat screen's files -- it is not a chat until its first
 * message is sent, and any list refresh before that would show it as one. */
function retainedConversations(conversations: ChatConversationSummary[]): ChatConversationSummary[] {
  return conversations.filter(
    (conversation) =>
      !removedConversations.has(conversation.conversation_id) &&
      conversation.conversation_id !== state.reservedConversationId,
  );
}

export function removeConversationFromStore(conversationId: string): void {
  removedConversations.add(conversationId);
  removeRetainedLive(conversationId);
  if (state.conversationId === conversationId) beginNewConversation();
  setState({ conversations: retainedConversations(state.conversations) });
}
const listeners = new Set<Listener>();
let unlisteners: UnlistenFn[] = [];
let started = false;
let bridgeStart: Promise<void> | null = null;

/**
 * Invocation ids that belong to the active conversation's run. Tool events
 * after `requested` carry only `invocation_id`, so membership here is how a
 * follow-up (`approval_required`, `completed`, ...) proves it is ours and not
 * from a run streaming in another conversation. Mirrors xfalcon-web, where
 * every store slice is keyed by conversation.
 */
const admittedInvocations = new Set<string>();

/**
 * Live run data has no durable replay source. Keep a small process-local cache
 * so leaving a conversation does not make an adopted run deaf to the broadcast
 * event channels. Ownership indexes are populated only at an authoritative
 * adoption seam; an arbitrary background run can never create an entry.
 */
type RetainedLiveState = {
  conversationId: string;
  /** The run allowed to mutate this entry. Null after durable retirement. */
  runId: string | null;
  /** Highest durable message sequence seen when this entry was retained. */
  messageSequence: bigint;
  activeRunId: string | null;
  runState: string | null;
  streamingText: string;
  settlingText: string;
  compactingRunId: string | null;
  recoveryNotice: string | null;
  error: string | null;
  activity: ToolInvocationEvent[];
  roundProse: LiveSegment[];
  reasoning: LiveSegment[];
  reasoningTrimmed: boolean;
  admittedInvocations: Set<string>;
};

const MAX_RETAINED_LIVE_CONVERSATIONS = 12;
const retainedLiveByConversation = new Map<string, RetainedLiveState>();
const conversationByRun = new Map<string, string>();
const conversationByInvocation = new Map<string, string>();
let pendingConversationOpenRestore: ConversationOpenRestore | null = null;

/** A scheduled coalesced notification, if one is in flight. */
let pendingEmitFrame: number | null = null;

function emit() {
  if (pendingEmitFrame !== null) {
    cancelAnimationFrame(pendingEmitFrame);
    pendingEmitFrame = null;
  }
  for (const listener of listeners) {
    listener();
  }
}

function setState(patch: Partial<ChatStoreSnapshot>) {
  // An empty patch is the reducers' "this event is not ours" answer, and the
  // inference channel is a broadcast — a run streaming in another conversation
  // produces one per token. Committing a fresh state object for those would
  // re-render the whole transcript at that run's token rate.
  if (Object.keys(patch).length === 0) return;
  state = { ...state, ...patch };
  syncTurnClock(state.awaitingRun || isBoundRunPending(state.activeRunId, state.runState), state.activeRunId);
  emit();
}

/**
 * Commit state now, notify subscribers at most once per frame.
 *
 * Streamed text arrives one token at a time; notifying on every token
 * re-renders the whole transcript — markdown re-parse of the growing answer
 * included — tens of times a second, which is the difference between a
 * transcript that glides and one that stutters. `state` is still written
 * synchronously, so `getChatSnapshot` and the round-boundary capture in
 * `applyToolInvocation` read exact values; only the React notification
 * coalesces. Mirrors xfalcon-web's rAF delta flush
 * (`useSendMessage.scheduleDeltaFlush`).
 */
function setStateCoalesced(patch: Partial<ChatStoreSnapshot>) {
  state = { ...state, ...patch };
  syncTurnClock(state.awaitingRun || isBoundRunPending(state.activeRunId, state.runState), state.activeRunId);
  if (pendingEmitFrame !== null) return;
  pendingEmitFrame = requestAnimationFrame(() => {
    pendingEmitFrame = null;
    for (const listener of listeners) {
      listener();
    }
  });
}

function maxMessageSequence(messages: Array<{ sequence: bigint }>): bigint {
  let max = BigInt(0);
  for (const message of messages) {
    if (message.sequence > max) {
      max = message.sequence;
    }
  }
  return max;
}

function cloneRetainedLive(entry: RetainedLiveState): RetainedLiveState {
  return {
    ...entry,
    activity: [...entry.activity],
    roundProse: [...entry.roundProse],
    reasoning: [...entry.reasoning],
    admittedInvocations: new Set(entry.admittedInvocations),
  };
}

function clearRetainedOwnership(entry: RetainedLiveState): void {
  if (entry.runId !== null && conversationByRun.get(entry.runId) === entry.conversationId) {
    conversationByRun.delete(entry.runId);
    forgetInferenceSequenceState(entry.runId);
  }
  for (const invocationId of entry.admittedInvocations) {
    if (conversationByInvocation.get(invocationId) === entry.conversationId) {
      conversationByInvocation.delete(invocationId);
    }
  }
}

function removeRetainedLive(conversationId: string): void {
  const existing = retainedLiveByConversation.get(conversationId);
  if (existing === undefined) return;
  clearRetainedOwnership(existing);
  retainedLiveByConversation.delete(conversationId);
}

function putRetainedLive(entry: RetainedLiveState): void {
  const previous = retainedLiveByConversation.get(entry.conversationId);
  if (previous !== undefined && previous.runId !== entry.runId) {
    clearRetainedOwnership(previous);
  }
  retainedLiveByConversation.delete(entry.conversationId);
  retainedLiveByConversation.set(entry.conversationId, entry);
  if (entry.runId !== null) conversationByRun.set(entry.runId, entry.conversationId);
  if (
    previous === undefined ||
    previous.runId !== entry.runId ||
    previous.admittedInvocations !== entry.admittedInvocations
  ) {
    for (const invocationId of entry.admittedInvocations) {
      conversationByInvocation.set(invocationId, entry.conversationId);
    }
  }
  while (retainedLiveByConversation.size > MAX_RETAINED_LIVE_CONVERSATIONS) {
    let oldestRetiredConversationId: string | undefined;
    let oldestConversationId: string | undefined;
    for (const [conversationId, retained] of retainedLiveByConversation) {
      if (conversationId === state.conversationId) continue;
      oldestConversationId ??= conversationId;
      if (retained.runId === null) {
        oldestRetiredConversationId = conversationId;
        break;
      }
    }
    const evictedConversationId = oldestRetiredConversationId ?? oldestConversationId;
    if (evictedConversationId === undefined) break;
    removeRetainedLive(evictedConversationId);
  }
}

function retainedPatch(entry: RetainedLiveState): Partial<ChatStoreSnapshot> {
  return {
    activeRunId: entry.activeRunId,
    runState: entry.runState,
    streamingText: entry.streamingText,
    settlingText: entry.settlingText,
    compactingRunId: entry.compactingRunId,
    recoveryNotice: entry.recoveryNotice,
    error: entry.error,
    activity: entry.activity,
    roundProse: entry.roundProse,
    reasoning: entry.reasoning,
    reasoningTrimmed: entry.reasoningTrimmed,
  };
}

function snapshotForRetained(entry: RetainedLiveState): ChatStoreSnapshot {
  return {
    ...state,
    conversationId: entry.conversationId,
    ...retainedPatch(entry),
    // `...state` carries the CURRENT conversation's `revising`. A retained entry
    // never stores one, and letting it through would hand an edit or regenerate
    // in progress in another conversation to this one, where nothing could end it.
    revising: false,
  };
}

function retainCurrentLive(runIdHint?: string | null): void {
  const conversationId = state.conversationId;
  if (conversationId === null) return;
  if (removedConversations.has(conversationId)) {
    removeRetainedLive(conversationId);
    return;
  }
  const previous = retainedLiveByConversation.get(conversationId);
  const runId =
    runIdHint === undefined
      ? (previous?.runId ?? (isBoundRunPending(state.activeRunId, state.runState) ? state.activeRunId : null))
      : runIdHint;
  const hasLiveData =
    runId !== null ||
    state.streamingText.length > 0 ||
    state.settlingText.length > 0 ||
    state.activity.length > 0 ||
    state.roundProse.length > 0 ||
    state.reasoning.length > 0 ||
    state.compactingRunId !== null;
  if (!hasLiveData) {
    removeRetainedLive(conversationId);
    return;
  }
  putRetainedLive({
    conversationId,
    runId,
    messageSequence: maxMessageSequence(state.messages),
    activeRunId: state.activeRunId,
    runState: state.runState,
    streamingText: state.streamingText,
    settlingText: state.settlingText,
    compactingRunId: state.compactingRunId,
    recoveryNotice: state.recoveryNotice,
    error: state.error,
    activity: state.activity,
    roundProse: state.roundProse,
    reasoning: state.reasoning,
    reasoningTrimmed: state.reasoningTrimmed,
    admittedInvocations: new Set(admittedInvocations),
  });
}

function restoreAdmittedInvocations(entry: RetainedLiveState | undefined): void {
  admittedInvocations.clear();
  if (entry === undefined) return;
  for (const invocationId of entry.admittedInvocations) admittedInvocations.add(invocationId);
}

function retireRetainedRun(conversationId: string, reasoning: LiveSegment[], reasoningTrimmed: boolean): void {
  const previous = retainedLiveByConversation.get(conversationId);
  if (previous !== undefined) clearRetainedOwnership(previous);
  if (reasoning.length === 0) {
    retainedLiveByConversation.delete(conversationId);
    return;
  }
  putRetainedLive({
    conversationId,
    runId: null,
    messageSequence: previous?.messageSequence ?? BigInt(0),
    activeRunId: null,
    runState: null,
    streamingText: "",
    settlingText: "",
    compactingRunId: null,
    recoveryNotice: null,
    error: null,
    activity: [],
    roundProse: [],
    reasoning,
    reasoningTrimmed,
    admittedInvocations: new Set(),
  });
}

function updateRetainedLive(entry: RetainedLiveState, patch: Partial<ChatStoreSnapshot>): RetainedLiveState {
  const next = { ...snapshotForRetained(entry), ...patch };
  const updated: RetainedLiveState = {
    ...entry,
    activeRunId: next.activeRunId,
    runState: next.runState,
    streamingText: next.streamingText,
    settlingText: next.settlingText,
    compactingRunId: next.compactingRunId,
    recoveryNotice: next.recoveryNotice,
    error: next.error,
    activity: next.activity,
    roundProse: next.roundProse,
    reasoning: next.reasoning,
    reasoningTrimmed: next.reasoningTrimmed,
  };
  putRetainedLive(updated);
  return updated;
}

function toolInvocationPatch(snapshot: ChatStoreSnapshot, event: ToolInvocationEvent): Partial<ChatStoreSnapshot> {
  const next = [...snapshot.activity, event];
  const patch: Partial<ChatStoreSnapshot> = {
    activity: next.length > MAX_ACTIVITY_EVENTS ? next.slice(next.length - MAX_ACTIVITY_EVENTS) : next,
  };
  if (event.type === "requested" && snapshot.streamingText.trim().length > 0) {
    patch.roundProse = [
      ...snapshot.roundProse,
      { text: snapshot.streamingText, afterCallCount: toolRowCount(snapshot.activity) },
    ];
    patch.streamingText = "";
  }
  return patch;
}

/**
 * The sidebar and palette read `conversations`, not `title`. A snapshot that
 * upgrades the open chat's name used to leave those rows on the derived opener,
 * so the titlebar said one thing and Recents another.
 */
function conversationListWithTitle(
  conversations: ChatConversationSummary[],
  conversationId: string,
  title: string,
): ChatConversationSummary[] | undefined {
  const index = conversations.findIndex((conversation) => conversation.conversation_id === conversationId);
  if (index < 0) return undefined;
  const current = conversations[index];
  if (current === undefined || current.title === title) return undefined;
  const next = conversations.slice();
  next[index] = { ...current, title };
  return next;
}

function conversationListTitleDiffers(conversationId: string, title: string): boolean {
  const row = state.conversations.find((conversation) => conversation.conversation_id === conversationId);
  return row === undefined || row.title !== title;
}

/**
 * Reloads the workspace conversation list — the same fetch F5 runs.
 *
 * Local list patching covers the open row immediately. This fetch is what
 * makes every other row (and a reload) agree, because the sidebar is backed
 * by the workspace collection, not by the open snapshot.
 */
function scheduleWorkspaceRefresh(): void {
  if (!(globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__) return;
  const accountGeneration = getAuthAccountGeneration();
  void import("../../lib/native")
    .then(({ chatWorkspaceBootstrap }) => chatWorkspaceBootstrap())
    .then((workspace) => {
      if (accountGeneration !== getAuthAccountGeneration()) return;
      setWorkspaceBootstrap(workspace.conversations);
    })
    .catch(() => {
      // The patched title is already on the row; a failed refresh must not
      // take it away.
    });
}

function mapConversationView(
  view: ChatConversationView,
  keepsLiveRun: boolean,
  liveSource: ChatStoreSnapshot,
): Partial<ChatStoreSnapshot> {
  // Re-applying the conversation already on screen (a mid-run reconcile, a
  // rename) must not knock over live run state: the streaming bubble and the
  // activity feed (which carries the tool-approval prompt) exist only in this
  // store, so the snapshot cannot restore what wiping them loses.
  const sameConversation = liveSource.conversationId === view.conversation_id;
  const keepsRetryTarget =
    sameConversation &&
    liveSource.activeRunId !== null &&
    (view.active_run_id === null || view.active_run_id === liveSource.activeRunId) &&
    (liveSource.runState === "failed" || liveSource.runState === "interrupted");
  return {
    conversationId: view.conversation_id,
    title: view.title,
    availability: view.availability,
    messages: view.messages.map((message) => ({
      id: message.message_id,
      role: message.role === "tool" ? "system" : message.role,
      text: message.text,
      sequence: message.sequence,
      toolCalls: message.tool_calls,
      attachments: message.attachments,
    })),
    // A terminal snapshot retires a run that the renderer still calls active.
    // Keep only a failed/interrupted run's id because that id is also the
    // retry target. The former unconditional fallback kept Stop alive after
    // the backend had already settled a cancelled run.
    activeRunId: keepsLiveRun
      ? liveSource.activeRunId
      : (view.active_run_id ?? (keepsRetryTarget ? liveSource.activeRunId : null)),
    runState: keepsLiveRun || keepsRetryTarget ? liveSource.runState : null,
    recoveryNotice: sameConversation ? liveSource.recoveryNotice : null,
    error: sameConversation ? liveSource.error : null,
    // Every live surface of a run retires together, on this one signal — see
    // `applyConversation`. Retiring them separately is what let the streamed
    // answer and its durable row be on screen at the same time.
    streamingText: keepsLiveRun ? liveSource.streamingText : "",
    settlingText: keepsLiveRun ? liveSource.settlingText : "",
    compactingRunId: keepsLiveRun ? liveSource.compactingRunId : null,
    activity: keepsLiveRun ? liveSource.activity : [],
    roundProse: keepsLiveRun ? liveSource.roundProse : [],
    // Exempt from the one-signal retirement above, and scoped to the
    // conversation instead: the snapshot retires what it carries, and it
    // carries no reasoning. A conversation switch keeps it off the destination
    // view; the retained source entry restores it only when that conversation
    // returns, so one run's thinking never attaches to another answer.
    reasoning: sameConversation ? liveSource.reasoning : [],
    reasoningTrimmed: sameConversation ? liveSource.reasoningTrimmed : false,
    // A revision outlives the snapshot of its own conversation: the backend
    // emits one right after it truncates the transcript, while the edit or
    // regenerate command is still awaiting its run id. Clearing the claim here
    // left no run, no submission and no revision for that gap, so the pill
    // vanished and came back when the command returned. `finishTurnRevision`
    // and `rollbackTurnRevision` own the end of a revision; a switch to another
    // conversation still drops it, because it belongs to the one it began in.
    revising: sameConversation ? liveSource.revising : false,
  };
}

function applyConversation(view: ChatConversationView, authoritative = true) {
  if (removedConversations.has(view.conversation_id)) return;
  const switchingConversation = state.conversationId !== view.conversation_id;
  if (switchingConversation) retainCurrentLive();
  const retained = retainedLiveByConversation.get(view.conversation_id);
  const liveSource = switchingConversation && retained !== undefined ? snapshotForRetained(retained) : state;
  // Live run state — the streamed/settling answer, the activity feed, the
  // captured narration, and the admitted-invocation set that scopes them —
  // belongs to an IN-FLIGHT run.
  // It survives only a mid-run reconcile of the conversation on screen; any
  // other snapshot is the durable record that supersedes it, and the tool
  // round it carries renders as the umbrella above the answer. Keeping the
  // live card past that point drew the same round twice, once above the
  // answer and once below it (spec: chat-run-activity).
  //
  // Retiring on the snapshot's arrival rather than on the run's falling edge
  // is deliberate: `finish_run` reconciles after the terminal run event, with
  // retries, so clearing early would blank the card for that gap.
  //
  // "In flight" is a run bound to the view that has not been observed to
  // settle — `runState` is still null when a conversation opens onto a run
  // that is already running (startup recovery, a background run), and that
  // run's activity has to survive the open.
  const hasLiveRun = isBoundRunPending(liveSource.activeRunId, liveSource.runState);
  const keepsLiveRun =
    liveSource.conversationId === view.conversation_id &&
    ((view.active_run_id !== null && view.active_run_id === liveSource.activeRunId && hasLiveRun) ||
      (!authoritative && retained !== undefined && retained.runId !== null));
  if (switchingConversation) {
    clearTurnEdit();
  }
  restoreAdmittedInvocations(keepsLiveRun ? retained : undefined);
  const conversations = conversationListWithTitle(state.conversations, view.conversation_id, view.title);
  setState({
    ...mapConversationView(view, keepsLiveRun, liveSource),
    ...(conversations !== undefined ? { conversations } : {}),
  });
  if (keepsLiveRun) {
    retainCurrentLive(retained?.runId ?? view.active_run_id);
  } else {
    retireRetainedRun(view.conversation_id, state.reasoning, state.reasoningTrimmed);
    if (view.active_run_id !== null) {
      retainCurrentLive(view.active_run_id);
    }
  }
}

function reconcilePendingOpenRestore(view: ChatConversationView): void {
  const restore = pendingConversationOpenRestore;
  if (restore === null || restore.snapshot.conversationId !== view.conversation_id) return;
  if (maxMessageSequence(view.messages) < maxMessageSequence(restore.snapshot.messages)) return;

  const retained = retainedLiveByConversation.get(view.conversation_id);
  const liveSource =
    retained === undefined || retained.runId === null
      ? restore.snapshot
      : { ...restore.snapshot, conversationId: view.conversation_id, ...retainedPatch(retained) };
  const keepsLiveRun =
    view.active_run_id !== null &&
    view.active_run_id === liveSource.activeRunId &&
    isBoundRunPending(liveSource.activeRunId, liveSource.runState);
  restore.snapshot = copySnapshot({
    ...restore.snapshot,
    ...mapConversationView(view, keepsLiveRun, liveSource),
    conversations: state.conversations,
  });
  restore.admittedInvocations = keepsLiveRun && retained !== undefined ? Array.from(retained.admittedInvocations) : [];
}

function applyProjection(event: ChatProjectionEvent) {
  if (event.type === "conversation_removed") {
    const session = getAuthSession();
    if (
      session.status !== "authenticated" ||
      `${session.identity.tenant_id}:${session.identity.user_id}` !== event.account_id
    )
      return;
    removeConversationFromStore(event.conversation_id);
    return;
  }
  if (event.type === "snapshot_replaced") {
    const view = event.conversation;
    if (removedConversations.has(view.conversation_id)) return;
    // Title is list metadata, not the open transcript. An LLM title upgrade
    // (and a rename) must reach Recents even when that chat is not on screen.
    // F5 already showed the workspace collection has the model title; reload
    // that collection here so Recents does not wait for a full page refresh.
    if (conversationListTitleDiffers(view.conversation_id, view.title)) {
      scheduleWorkspaceRefresh();
    }
    const conversations = conversationListWithTitle(state.conversations, view.conversation_id, view.title);
    if (conversations !== undefined && view.conversation_id !== state.conversationId) {
      setState({ conversations });
    }
    // Only the conversation on screen may be replaced by a broadcast. A
    // background run finalizing elsewhere emits the same event, and adopting
    // its snapshot would teleport the user into that conversation (the
    // "response from another chat shows up in my new chat" failure).
    if (view.conversation_id !== state.conversationId) {
      reconcilePendingOpenRestore(view);
      const retained = retainedLiveByConversation.get(view.conversation_id);
      if (retained !== undefined && maxMessageSequence(view.messages) >= retained.messageSequence) {
        const keepsRetainedRun =
          view.active_run_id !== null &&
          view.active_run_id === retained.activeRunId &&
          isBoundRunPending(retained.activeRunId, retained.runState);
        if (keepsRetainedRun) {
          putRetainedLive({ ...retained, messageSequence: maxMessageSequence(view.messages) });
        } else {
          retireRetainedRun(view.conversation_id, retained.reasoning, retained.reasoningTrimmed);
        }
      }
      return;
    }
    if (maxMessageSequence(view.messages) < maxMessageSequence(state.messages)) {
      if (conversations !== undefined) {
        setState({ conversations });
      }
      return;
    }
    applyConversation(view);
    // The preview command emits nothing. Any admitted projection snapshot for
    // the opening target is therefore authoritative native state and settles
    // navigation before a slower preview promise can repaint stale content.
    if (state.openingConversationId === view.conversation_id) {
      pendingConversationOpenRestore = null;
      setState({ openingConversationId: null, conversationContentPending: false });
    }
    return;
  }
  if (event.type === "run_state_changed") {
    // Conversation-scoped run announcement: this is how the store learns
    // which run belongs to the conversation it is showing. Everything on the
    // run/inference/tool channels is then admitted by that run id.
    if (event.conversation_id !== state.conversationId) {
      const retained = retainedLiveByConversation.get(event.conversation_id);
      if (
        retained === undefined ||
        retained.runId !== event.run_id ||
        conversationByRun.get(event.run_id) !== event.conversation_id
      ) {
        return;
      }
      updateRetainedLive(
        retained,
        reduceRunEvent(snapshotForRetained(retained), {
          type: "state_changed",
          version: event.version,
          run_id: event.run_id,
          state: event.state,
        }),
      );
      return;
    }
    const announcesActiveRun = isBoundRunPending(event.run_id, event.state);
    if (event.run_id !== state.activeRunId && !announcesActiveRun) {
      // The native open path emits its durable snapshot before the latest run
      // state. A duplicate terminal announcement must not resurrect that run
      // or erase the reasoning the snapshot deliberately preserved.
      if (state.activeRunId !== null || event.state === "completed" || event.state === "cancelled") return;
    }
    if (event.run_id !== state.activeRunId && announcesActiveRun) {
      removeRetainedLive(event.conversation_id);
      admittedInvocations.clear();
      setState({
        activeRunId: event.run_id,
        runState: null,
        streamingText: "",
        settlingText: "",
        compactingRunId: null,
        activity: [],
        roundProse: [],
        reasoning: [],
        reasoningTrimmed: false,
        recoveryNotice: null,
        error: null,
      });
    }
    setState({
      ...reduceRunEvent(
        { ...state, activeRunId: event.run_id },
        {
          type: "state_changed",
          version: event.version,
          run_id: event.run_id,
          state: event.state,
        },
      ),
      ...(event.run_id !== state.activeRunId ? { error: null } : {}),
    });
    retainCurrentLive(event.run_id);
  }
}

function applyRunEvent(event: RunEvent) {
  const conversationId = conversationByRun.get(event.run_id);
  if (event.run_id === state.activeRunId && (conversationId === undefined || conversationId === state.conversationId)) {
    setState(reduceRunEvent(state, event));
    if (state.conversationId !== null) retainCurrentLive(event.run_id);
    return;
  }
  if (conversationId === undefined) return;
  const retained = retainedLiveByConversation.get(conversationId);
  if (retained === undefined || retained.runId !== event.run_id) return;
  updateRetainedLive(retained, reduceRunEvent(snapshotForRetained(retained), event));
}

export function applyRunEventForTests(event: RunEvent): void {
  applyRunEvent(event);
}

function applyInference(event: InferenceEvent) {
  const conversationId = conversationByRun.get(event.run_id);
  if (event.run_id !== state.activeRunId || (conversationId !== undefined && conversationId !== state.conversationId)) {
    if (conversationId === undefined) return;
    const retained = retainedLiveByConversation.get(conversationId);
    if (retained === undefined || retained.runId !== event.run_id) return;
    const patch = reduceInferenceEvent(snapshotForRetained(retained), event);
    if (Object.keys(patch).length > 0) updateRetainedLive(retained, patch);
    return;
  }
  const patch = reduceInferenceEvent(state, event);
  // Only the token streams coalesce; terminal events (`completed`, `failed`)
  // hand off to the durable transcript and must land on the very next render.
  // Reasoning arrives at the same token cadence as text, so it takes the same
  // route — a synchronous commit per fragment re-renders the whole transcript,
  // markdown re-parse of the growing answer included, once per token.
  if (event.type === "text_delta" && patch.streamingText !== undefined) {
    setStateCoalesced(patch);
    return;
  }
  if (event.type === "reasoning_summary" && patch.reasoning !== undefined) {
    setStateCoalesced(patch);
    return;
  }
  setState(patch);
}

function applyToolInvocation(event: ToolInvocationEvent) {
  // Only `requested` names its run; follow-ups carry just the invocation id.
  // Admit a `requested` for an explicitly adopted run, then admit follow-ups
  // only for invocations already admitted. A background conversation updates
  // only its retained entry, so its approval prompt never reaches this view.
  let conversationId: string | undefined;
  let retained: RetainedLiveState | undefined;
  if (event.type === "requested") {
    conversationId = conversationByRun.get(event.run_id);
    if (conversationId === undefined) return;
    retained = retainedLiveByConversation.get(conversationId);
    if (retained === undefined || retained.runId !== event.run_id) return;
    retained = {
      ...retained,
      admittedInvocations: new Set(retained.admittedInvocations).add(event.invocation_id),
    };
    conversationByInvocation.set(event.invocation_id, conversationId);
    putRetainedLive(retained);
  } else {
    conversationId = conversationByInvocation.get(event.invocation_id);
    if (conversationId === undefined) return;
    retained = retainedLiveByConversation.get(conversationId);
    if (retained === undefined || !retained.admittedInvocations.has(event.invocation_id)) return;
  }

  if (conversationId !== state.conversationId) {
    updateRetainedLive(retained, toolInvocationPatch(snapshotForRetained(retained), event));
    return;
  }
  if (event.type === "requested") admittedInvocations.add(event.invocation_id);
  if (!admittedInvocations.has(event.invocation_id)) return;
  setState(toolInvocationPatch(state, event));
}

export function applyToolInvocationForTests(event: ToolInvocationEvent): void {
  applyToolInvocation(event);
}

export function applyInferenceForTests(event: InferenceEvent): void {
  applyInference(event);
}

export function getChatSnapshot(): ChatStoreSnapshot {
  return state;
}

export function subscribeChat(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/**
 * Subscribes to ONE value of the snapshot.
 *
 * A component that reads the whole snapshot re-renders for every field, which is
 * how a streamed token came to re-render the entire application. Selecting the
 * field means React compares that value and nothing else
 * (spec: chat-streaming-performance).
 *
 * `select` MUST return a primitive or a reference this store owns. React calls
 * it on every notification and compares the result by reference, so a selector
 * that builds a fresh object or array reports a change every time and re-renders
 * forever. `chatSelectors.test.ts` holds that line for the selectors shipped
 * here.
 */
export function useChatSelector<T>(select: (snapshot: ChatStoreSnapshot) => T): T {
  const read = () => select(state);
  return useSyncExternalStore(subscribeChat, read, read);
}

export function resetChatStore() {
  removedConversations.clear();
  resetInferenceSequenceState();
  admittedInvocations.clear();
  retainedLiveByConversation.clear();
  conversationByRun.clear();
  conversationByInvocation.clear();
  pendingConversationOpenRestore = null;
  // The draft lives in its own store now, but signing out must still leave none
  // of the previous account's typing behind — the guarantee it had while it was
  // a field of this snapshot.
  resetComposerDraft();
  resetUploadQueue();
  clearTurnEdit();
  state = { ...initialState };
  // Set directly rather than through `setState`, so the turn's clock is
  // cleared here: a stopwatch left running would carry the old account's start
  // into the next account's first run.
  resetTurnClock();
  emit();
}

export function setChatStoreForTests(override: Partial<ChatStoreSnapshot>) {
  setState(override);
  if (state.conversationId !== null && state.activeRunId !== null) {
    retainCurrentLive(state.activeRunId);
  }
}

/** Records the conversation reserved for the new-chat screen's attachments. */
export function setReservedConversation(conversationId: string | null) {
  if (state.reservedConversationId === conversationId) return;
  state = { ...state, reservedConversationId: conversationId };
  setState({ conversations: retainedConversations(state.conversations) });
}

export function beginNewConversation() {
  retainCurrentLive();
  pendingConversationOpenRestore = null;
  admittedInvocations.clear();
  clearTurnEdit();
  setState({
    conversationId: null,
    title: "New conversation",
    availability: null,
    messages: [],
    streamingText: "",
    settlingText: "",
    compactingRunId: null,
    activeRunId: null,
    runState: null,
    error: null,
    openingConversationId: null,
    conversationContentPending: false,
    recoveryNotice: null,
    activity: [],
    roundProse: [],
    reasoning: [],
    reasoningTrimmed: false,
    revising: false,
    awaitingRun: false,
  });
}

export type ConversationOpenRestore = {
  targetId: string;
  snapshot: ChatStoreSnapshot;
  admittedInvocations: string[];
};

function copySnapshot(snapshot: ChatStoreSnapshot): ChatStoreSnapshot {
  return {
    ...snapshot,
    messages: [...snapshot.messages],
    conversations: [...snapshot.conversations],
    activity: [...snapshot.activity],
    roundProse: [...snapshot.roundProse],
    reasoning: [...snapshot.reasoning],
  };
}

/**
 * Selects a conversation before I/O and captures every field needed to undo
 * the optimistic switch without leaking either conversation's live run state.
 */
export function beginConversationOpen(
  targetId: string,
  continuingRestore: ConversationOpenRestore | null = null,
): ConversationOpenRestore | null {
  if (removedConversations.has(targetId)) return null;
  if (state.openingConversationId === targetId) return null;
  const restore =
    continuingRestore !== null && pendingConversationOpenRestore === continuingRestore
      ? continuingRestore
      : {
          targetId,
          snapshot: copySnapshot(state),
          admittedInvocations: Array.from(admittedInvocations),
        };
  // Consecutive selections are one optimistic navigation. Keep the original
  // settled source snapshot and its identity so a background authoritative
  // update can reconcile the same object that the hook will later roll back.
  restore.targetId = targetId;
  pendingConversationOpenRestore = restore;
  const knownTitle = state.conversations.find((item) => item.conversation_id === targetId)?.title;
  retainCurrentLive();
  const retained = retainedLiveByConversation.get(targetId);

  restoreAdmittedInvocations(retained);
  clearTurnEdit();
  setState({
    conversationId: targetId,
    title: knownTitle || "Untitled conversation",
    availability: null,
    messages: [],
    streamingText: "",
    settlingText: "",
    compactingRunId: null,
    activeRunId: null,
    runState: null,
    error: null,
    activity: [],
    roundProse: [],
    reasoning: [],
    reasoningTrimmed: false,
    revising: false,
    awaitingRun: false,
    openingConversationId: targetId,
    conversationContentPending: true,
    ...(retained === undefined ? {} : retainedPatch(retained)),
  });
  return restore;
}

export function applyConversationPreview(view: ChatConversationView, targetId: string): void {
  if (state.conversationId !== targetId || state.openingConversationId !== targetId) return;
  applyConversation(view, false);
  setState({ conversationContentPending: false });
}

export function completeConversationOpen(view: ChatConversationView, targetId: string): void {
  if (state.conversationId !== targetId || state.openingConversationId !== targetId) return;
  applyConversation(view);
  pendingConversationOpenRestore = null;
  setState({ openingConversationId: null, conversationContentPending: false });
}

export function rollbackConversationOpen(restore: ConversationOpenRestore | null): void {
  if (
    restore === null ||
    state.conversationId !== restore.targetId ||
    state.openingConversationId !== restore.targetId
  ) {
    return;
  }
  pendingConversationOpenRestore = null;
  if (restore.snapshot.conversationId != null && removedConversations.has(restore.snapshot.conversationId)) {
    beginNewConversation();
    return;
  }
  retainCurrentLive();
  const retained =
    restore.snapshot.conversationId === null
      ? undefined
      : retainedLiveByConversation.get(restore.snapshot.conversationId);
  if (retained === undefined) {
    admittedInvocations.clear();
    for (const id of restore.admittedInvocations) admittedInvocations.add(id);
  } else {
    restoreAdmittedInvocations(retained);
  }
  // A retired entry carries only reasoning. The reconciled restore snapshot
  // owns terminal and retry state; the cache's cleared fields cannot replace it.
  const livePatch =
    retained === undefined
      ? {}
      : retained.runId === null
        ? { reasoning: retained.reasoning, reasoningTrimmed: retained.reasoningTrimmed }
        : retainedPatch(retained);
  setState({
    ...copySnapshot(restore.snapshot),
    ...livePatch,
    conversations: retainedConversations(restore.snapshot.conversations),
  });
}

export type TurnRevisionRestore = {
  snapshot: ChatStoreSnapshot;
  admittedInvocations: string[];
  retainedLive: RetainedLiveState | null;
  inferenceSequence: { runId: string; snapshot: InferenceSequenceSnapshot } | null;
};

export function beginTurnRevision(cutoffMessageId: string): TurnRevisionRestore | null {
  // One optimistic cut owns one restore point. A second cut made from the
  // already-truncated state could only restore a partial transcript if either
  // command failed.
  if (state.revising || state.awaitingRun) {
    return null;
  }
  const index = state.messages.findIndex((m) => m.id === cutoffMessageId);
  if (index === -1) {
    return null;
  }
  retainCurrentLive();
  const retained = state.conversationId === null ? undefined : retainedLiveByConversation.get(state.conversationId);
  const sequenceRunId = retained?.runId ?? state.activeRunId;
  const restore: TurnRevisionRestore = {
    snapshot: {
      ...state,
      messages: [...state.messages],
      conversations: [...state.conversations],
      activity: [...state.activity],
      roundProse: [...state.roundProse],
      reasoning: [...state.reasoning],
    },
    admittedInvocations: Array.from(admittedInvocations),
    retainedLive: retained === undefined ? null : cloneRetainedLive(retained),
    inferenceSequence:
      sequenceRunId === null ? null : { runId: sequenceRunId, snapshot: captureInferenceSequenceState(sequenceRunId) },
  };
  const nextMessages = state.messages.slice(0, index);

  admittedInvocations.clear();
  if (state.conversationId !== null) removeRetainedLive(state.conversationId);
  setState({
    messages: nextMessages,
    streamingText: "",
    settlingText: "",
    compactingRunId: null,
    activeRunId: null,
    runState: null,
    activity: [],
    roundProse: [],
    reasoning: [],
    reasoningTrimmed: false,
    revising: true,
  });

  return restore;
}

export function rollbackTurnRevision(restore: TurnRevisionRestore | null): void {
  if (!restore) {
    setState({ revising: false });
    return;
  }
  // `editTurn`/`regenerateTurn` await the backend with no way to cancel a
  // conversation switch underneath them. If the reader has since moved on,
  // the open conversation is no longer the one this restore point belongs
  // to -- overwriting it with a stale snapshot would silently swap in
  // another conversation's messages, activity, and admitted-invocation set.
  // The switch itself already reset `revising` to false, so there is
  // nothing here left to undo.
  if (state.conversationId !== restore.snapshot.conversationId) {
    return;
  }
  admittedInvocations.clear();
  for (const id of restore.admittedInvocations) {
    admittedInvocations.add(id);
  }
  if (restore.retainedLive !== null) putRetainedLive(restore.retainedLive);
  if (restore.inferenceSequence !== null) {
    restoreInferenceSequenceState(restore.inferenceSequence.runId, restore.inferenceSequence.snapshot);
  }
  setState({ ...restore.snapshot, conversations: retainedConversations(restore.snapshot.conversations) });
}

export function finishTurnRevision(conversationId: string): void {
  // Same guard as `rollbackTurnRevision`: a late success for a conversation
  // the reader has since left must not touch whatever is open now, which
  // may be mid-revision of its own.
  if (state.revising && state.conversationId === conversationId) {
    setState({ revising: false });
  }
}

export function setWorkspaceBootstrap(conversations: ChatConversationSummary[], recoveryNotice?: string | null) {
  // Ordinary list refreshes do not carry a run identity and must leave an
  // existing notice alone. Startup recovery is explicit in the bootstrap
  // contract and is the one safe source allowed to surface it.
  setState({
    conversations: retainedConversations(conversations),
    workspaceLoaded: true,
    ...(recoveryNotice === undefined ? {} : { recoveryNotice }),
  });
}

export function applyConversationView(view: ChatConversationView) {
  applyConversation(view);
}

/**
 * Binds the conversation a just-submitted turn had to mint for itself.
 *
 * The row the reader typed is already on screen — it went up the instant they
 * pressed send, before this conversation existed — and the view that mints a
 * conversation carries no messages at all, because the message has not been
 * sent yet. Applying it as an ordinary snapshot would take that row away again
 * for the length of the send round trip, which is the exact window the
 * optimistic row exists to cover.
 *
 * Narrow on purpose: only this one hop, from no conversation to the one this
 * turn created, may carry an unrecorded row across a snapshot. Every later
 * snapshot for the conversation replaces the transcript wholesale, which is
 * what retires the optimistic row once the durable one arrives.
 */
export function applyCreatedConversationView(view: ChatConversationView) {
  if (removedConversations.has(view.conversation_id)) return;
  const optimistic = state.messages.filter((message) => message.id.startsWith(OPTIMISTIC_ROW_PREFIX));
  applyConversation(view);
  if (optimistic.length > 0) {
    setState({ messages: [...state.messages, ...optimistic] });
  }
}

/**
 * Claims a run for the turn being submitted, before one exists.
 *
 * Paired with `finishRunSubmission`, which every exit of the submit call has to
 * reach — a claim that outlives its round trip would leave the transcript
 * thinking about a run that was never minted.
 */
export function beginRunSubmission() {
  setState({ awaitingRun: true });
}

export function finishRunSubmission() {
  if (state.awaitingRun) {
    setState({ awaitingRun: false });
  }
}

/**
 * Binds a just-submitted run to the conversation on screen. The submit
 * result is the authoritative pairing (the backend minted the run for that
 * exact conversation); without this, every event the run emits before its
 * first conversation-scoped `run_state_changed` lands would be dropped as
 * foreign by the scoping in `applyRunEvent`/`applyInference`.
 */
export function adoptActiveRun(conversationId: string, runId: string) {
  if (removedConversations.has(conversationId)) {
    return;
  }
  const existing = retainedLiveByConversation.get(conversationId);
  if (existing?.runId === runId && (state.conversationId !== conversationId || state.activeRunId === runId)) {
    return;
  }
  removeRetainedLive(conversationId);
  if (state.conversationId !== conversationId) {
    putRetainedLive({
      conversationId,
      runId,
      messageSequence: BigInt(0),
      activeRunId: runId,
      runState: "queued",
      streamingText: "",
      settlingText: "",
      compactingRunId: null,
      recoveryNotice: null,
      error: null,
      activity: [],
      roundProse: [],
      reasoning: [],
      reasoningTrimmed: false,
      admittedInvocations: new Set(),
    });
    return;
  }
  admittedInvocations.clear();
  // A fresh run must not inherit the previous run's captured narration, nor
  // its thinking — reasoning outlives the durable snapshot precisely so it is
  // still readable here, which makes this the site that has to end it.
  setState({
    activeRunId: runId,
    runState: "queued",
    streamingText: "",
    settlingText: "",
    compactingRunId: null,
    recoveryNotice: null,
    error: null,
    activity: [],
    roundProse: [],
    reasoning: [],
    reasoningTrimmed: false,
  });
  retainCurrentLive(runId);
}

export function applyChatProjectionForTests(event: ChatProjectionEvent): void {
  applyProjection(event);
}

export function clearRecoveryNotice() {
  setState({ recoveryNotice: null });
}

/** Draws the turn the reader just sent, and returns the row's id so a
 * submission that never reached the backend can take it back. */
export function addOptimisticUserMessage(text: string, attachments: ChatAttachmentView[] = []): string {
  const next: ChatMessageRow = {
    id: `${OPTIMISTIC_ROW_PREFIX}${Date.now()}`,
    role: "user",
    text,
    sequence: BigInt(state.messages.length + 1),
    toolCalls: [],
    attachments,
  };
  setState({ messages: [...state.messages, next] });
  return next.id;
}

/**
 * Takes back a turn that never reached the backend.
 *
 * A row left behind by a failed submission has no durable counterpart to
 * replace it, so it would outlive the error that explains it — and, on the
 * new-conversation path, ride into the transcript of the next attempt.
 */
export function dropOptimisticUserMessage(id: string) {
  if (!state.messages.some((message) => message.id === id)) return;
  setState({ messages: state.messages.filter((message) => message.id !== id) });
}

export function setChatError(message: string | null) {
  setState({ error: message });
}

export function startChatEventBridge(): Promise<void> {
  if (bridgeStart !== null) return bridgeStart;
  bridgeStart = installChatEventBridge().finally(() => {
    bridgeStart = null;
  });
  return bridgeStart;
}

async function installChatEventBridge(): Promise<void> {
  if (started) return;
  if (!(globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__) {
    return;
  }
  started = true;
  try {
    unlisteners.push(
      await listen<ChatProjectionEvent>(CHAT_PROJECTION_EVENT_CHANNEL, (payload) => {
        applyProjection(payload.payload);
      }),
    );
    unlisteners.push(
      await listen<RunEvent>(CHAT_RUN_EVENT_CHANNEL, (payload) => {
        applyRunEvent(payload.payload);
      }),
    );
    unlisteners.push(
      await listen<InferenceEvent>(CHAT_INFERENCE_EVENT_CHANNEL, (payload) => {
        applyInference(payload.payload);
      }),
    );
    unlisteners.push(
      await listen<ToolInvocationEvent>(TOOL_INVOCATION_EVENT_CHANNEL, (payload) => {
        applyToolInvocation(payload.payload);
      }),
    );
  } catch (cause) {
    // A bridge that cannot listen means no streaming, no run states, and no
    // approval prompts — the exact "sent but nothing ever happens" failure.
    // Surface it and allow a retry instead of pretending the bridge is up.
    started = false;
    await stopChatEventBridge();
    setChatError(`Live updates are unavailable: ${cause instanceof Error ? cause.message : String(cause)}`);
    throw cause;
  }
}

export async function stopChatEventBridge(): Promise<void> {
  await Promise.all(unlisteners.map((unlisten) => unlisten()));
  unlisteners = [];
  started = false;
}
