import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { measureElement, useVirtualizer, type Virtualizer } from "@tanstack/react-virtual";
import { ArrowDown } from "lucide-react";
import { observeScrollRect } from "../../lib/virtualScrollRect";
import type { ChatToolCallView } from "../../lib/generated/contracts";
import { Tooltip } from "../../ui/Tooltip";
import { useShowReasoning } from "../../app/appearance";
import type { ChatMessageRow, LiveSegment } from "./chatStore";
import { CurrentAnswer, type LiveAnswer, type SavedAnswer } from "./CurrentAnswer";
import { MessageRow } from "./MessageRow";
import { responseAlreadyOpen } from "./runSurface";
import { ChatReportViews } from "../reports/ChatReportViews";
import { DurableToolTrail, REMEASURE_EVENT } from "./ToolActivityCard";

/** Returns to the newest message. Carries a visible label once a response has
 * landed, so only the bare-arrow state needs a tooltip to say what it does. */
function jump(showNewResponse: boolean, scrollToBottom: () => void) {
  const label = showNewResponse ? "Scroll to the new response" : "Scroll to latest message";
  const button = (
    <button
      aria-label={label}
      className={showNewResponse ? "type-caption chat-jump group px-4 py-2" : "chat-jump group justify-center p-2"}
      onClick={scrollToBottom}
      type="button"
    >
      <ArrowDown
        aria-hidden
        className="transition-transform duration-150 group-hover:translate-y-0.5"
        size={16}
        strokeWidth={1.8}
      />
      {showNewResponse ? "New response" : null}
    </button>
  );
  return showNewResponse ? button : <Tooltip label={label}>{button}</Tooltip>;
}

export type MessageListProps = {
  messages: ChatMessageRow[];
  /** The conversation these turns belong to, carried through to each row only
   * so an HTML document in an answer can name its own source when saved as a
   * report (spec: reports). */
  conversationId?: string | null;
  followStream?: boolean;
  /**
   * The settled run's reasoning, which the transcript draws once the live
   * timeline is gone.
   *
   * Reasoning is never persisted, so the durable record cannot carry it and the
   * live timeline cannot keep it — this is the handover between them. Empty
   * while the live timeline is on screen, or the same thinking would be drawn
   * twice (spec: chat-run-activity).
   */
  reasoning?: readonly LiveSegment[];
  /** The retention cap dropped the oldest of that reasoning. */
  reasoningTrimmed?: boolean;
  /** A live pill already owns the newest run's reasoning and tool events.
   * Hide the durable projection of that same run until the live pill retires,
   * so one agent run can never draw two activity disclosures. */
  suppressCurrentRunActivity?: boolean;
  /**
   * The run in flight, rendered inside the scroll flow after the last durable
   * item — where the next assistant turn will materialize.
   *
   * The virtualized window holds the durable conversation ONLY. Everything a
   * streamed token can move lives here instead, so a delta changes neither the
   * virtual row count nor any row key (spec:
   * chat-streaming-performance).
   */
  tail?: ReactNode;
  /** Settled-answer actions that follow both the live and durable run content. */
  footer?: ReactNode;
  /**
   * The live tail is showing this response's newest prose, so the accent rail
   * belongs there and not to any durable row (spec: chat-thread-layout, "Only
   * the newest answer is accented").
   */
  tailHasLatestAnswer?: boolean;
  /**
   * The run's answer while it is still the run's to show. The list draws it as
   * the SAME element that later draws the saved row, so the handover keeps the
   * rendered answer instead of rebuilding it (spec: chat-run-activity, "The
   * answer row survives the handover").
   */
  liveAnswer?: LiveAnswer | null;
  onEdit?: (messageId: string, text: string) => void;
  onRegenerate?: (messageId: string) => void;
  revisionPending?: boolean;
};

/** A stable empty list, so withholding reasoning does not re-key the memo that
 * reads it on every render. */
const NO_SEGMENTS: readonly LiveSegment[] = [];

const NEAR_BOTTOM_PX = 120;
const ESTIMATE_ROW_HEADER_PX = 44;
const ESTIMATE_CHARS_PER_LINE = 56;
const ESTIMATE_LINE_HEIGHT_PX = 22;
const ESTIMATE_MIN_ROW_PX = 72;
/** A collapsed disclosure is one summary row of card chrome — a CONSTANT,
 * deliberately: a run with forty steps and a run with one estimate the same
 * closed height, so the closed trail's footprint never grows with the run's
 * size (spec: chat-tool-trail, virtualization). The figure covers a step row's
 * two lines plus its chips, which the measured height then corrects. */
const ESTIMATE_DISCLOSURE_PX = 62;
/** The one key the current run's answer is drawn under, whichever source feeds it. */
const CURRENT_ANSWER_KEY = "current-answer";
/** How long a row keeps its entrance class. After this the id renders bare,
 * so a virtualization re-mount can never replay the animation. */
const ENTER_REPLAY_GUARD_MS = 400;

/**
 * What the transcript actually renders: one item per block of a run, in the
 * order the run produced it (spec: chat-tool-trail, chat-reasoning-panel).
 *
 * - **message** — a user turn or the final assistant answer.
 * - **activity** — every tool row and the run's reasoning, folded into one
 *   umbrella. Tool-round narration is loop progress, not another assistant
 *   response, so it does not split the umbrella or render as prose.
 */
type DisplayItem =
  | { kind: "message"; id: string; message: ChatMessageRow; opensResponse: boolean; isLastProse: boolean }
  | {
      kind: "activity";
      id: string;
      calls: ChatToolCallView[];
      reasoning?: { text: string; trimmed: boolean };
    };

/**
 * The first turn of the run in progress: one past the last user message, or 0
 * when there is none.
 *
 * This is the split point the whole transcript build turns on. A tool group is
 * always closed by a user message, so the boundary is also a clean group
 * boundary — which is what lets the turns above it be built once and only the
 * run's own turns rebuild as reasoning arrives
 * (spec: chat-streaming-performance).
 *
 * It is also where a response begins, which is what makes the ribbon decidable
 * per range: the settled range ends WITH that user message, so the run range
 * always starts with no response open.
 */
function runStartIndex(messages: ChatMessageRow[]): number {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    if (messages[index].role === "user") return index + 1;
  }
  return 0;
}

/**
 * Builds the display items for `messages[from, to)`.
 *
 * Reasoning describes the newest run only — the store clears it the moment a new
 * one is adopted — so a caller building the settled turns above `runStartIndex`
 * passes none. That is why there is no in-run test in the loop: the range IS the
 * test, and letting reasoning reach an earlier answer would file one run's
 * thinking under another run's tools.
 */
function buildDisplayItems(
  messages: ChatMessageRow[],
  from: number,
  to: number,
  reasoning: readonly LiveSegment[],
  reasoningTrimmed: boolean,
): DisplayItem[] {
  const items: DisplayItem[] = [];
  const open: { group: { id: string; calls: ChatToolCallView[] } | null } = { group: null };
  let reasoningCursor = 0;
  let responseOpen = false;
  let lastAssistantMsgIndex: number | null = null;

  const markLastAssistant = () => {
    if (lastAssistantMsgIndex !== null && items[lastAssistantMsgIndex]?.kind === "message") {
      (items[lastAssistantMsgIndex] as { isLastProse: boolean }).isLastProse = true;
      lastAssistantMsgIndex = null;
    }
  };

  const getReasoningUpTo = (callCount: number): { text: string; trimmed: boolean } | undefined => {
    const texts: string[] = [];
    let trimmed = false;
    while (reasoningCursor < reasoning.length && reasoning[reasoningCursor].afterCallCount <= callCount) {
      texts.push(reasoning[reasoningCursor].text);
      if (reasoningTrimmed && reasoningCursor === 0) trimmed = true;
      reasoningCursor += 1;
    }
    if (texts.length === 0) return undefined;
    return { text: texts.join("\n\n"), trimmed };
  };

  const flush = () => {
    if (open.group !== null) {
      const r = getReasoningUpTo(Number.MAX_SAFE_INTEGER);
      items.push({ kind: "activity", id: `activity-${open.group.id}`, calls: open.group.calls, reasoning: r });
      open.group = null;
    }
  };

  const openGroup = (id: string) => {
    const existing = open.group;
    if (existing !== null) return existing;
    const created = { id, calls: [] as ChatToolCallView[] };
    open.group = created;
    return created;
  };

  const pushMessage = (message: ChatMessageRow) => {
    if (message.role === "user") {
      markLastAssistant();
      responseOpen = false;
      items.push({ kind: "message", id: message.id, message, opensResponse: false, isLastProse: false });
      return;
    }
    const opensResponse = message.role === "assistant" && !responseOpen;
    if (opensResponse) responseOpen = true;
    const itemIndex = items.length;
    items.push({ kind: "message", id: message.id, message, opensResponse, isLastProse: false });
    lastAssistantMsgIndex = itemIndex;
  };

  for (let index = from; index < to; index += 1) {
    const message = messages[index];
    const isAssistant = message.role === "assistant";
    const hasTools = message.toolCalls.length > 0;
    const hasText = message.text.trim().length > 0;

    if (isAssistant && hasTools) {
      const group = openGroup(`tools-${message.id}`);
      group.calls.push(...message.toolCalls);
      continue;
    }

    if (!hasText && !hasTools) continue;

    // About to push prose / message: flush any pending tools (and reasoning) first
    flush();

    // If there is reasoning for this turn with no tools before it:
    const remainingReasoning = getReasoningUpTo(Number.MAX_SAFE_INTEGER);
    if (remainingReasoning) {
      items.push({
        kind: "activity",
        id: `activity-reasoning-${items.length}`,
        calls: [],
        reasoning: remainingReasoning,
      });
    }

    pushMessage(message);
  }

  flush();
  const trailingReasoning = getReasoningUpTo(Number.MAX_SAFE_INTEGER);
  if (trailingReasoning) {
    items.push({
      kind: "activity",
      id: `activity-reasoning-${items.length}`,
      calls: [],
      reasoning: trailingReasoning,
    });
  }
  markLastAssistant();
  return items;
}

function estimateItemHeight(item: DisplayItem | undefined): number {
  if (!item) return ESTIMATE_MIN_ROW_PX;
  if (item.kind === "activity") return ESTIMATE_DISCLOSURE_PX;
  const bodyLines = Math.max(1, Math.ceil(item.message.text.length / ESTIMATE_CHARS_PER_LINE));
  return Math.max(ESTIMATE_MIN_ROW_PX, ESTIMATE_ROW_HEADER_PX + bodyLines * ESTIMATE_LINE_HEIGHT_PX);
}

function measureMessageRow(
  element: Element,
  entry: ResizeObserverEntry | undefined,
  instance: Virtualizer<HTMLDivElement, Element>,
): number {
  const index = instance.indexFromElement(element);
  const key = instance.options.getItemKey(index);
  // The library reuses its cached size when no observer entry is supplied.
  // An explicit settle/remount measurement must read the current layout.
  const measured = entry ? measureElement(element, entry, instance) : element.getBoundingClientRect().height;
  const cached = instance.itemSizeCache.get(key);
  // Streaming rows live outside the virtual window. Historical rows must also
  // shrink when fonts, reading size, or disclosure content change. Only a
  // temporarily unmeasurable row retains its previous height.
  return measured > 0 ? measured : (cached ?? instance.options.estimateSize(index));
}

export function MessageList({
  messages,
  conversationId = null,
  followStream = false,
  reasoning = NO_SEGMENTS,
  reasoningTrimmed = false,
  suppressCurrentRunActivity = false,
  tail,
  footer,
  tailHasLatestAnswer = false,
  liveAnswer = null,
  onEdit,
  onRegenerate,
  revisionPending = false,
}: MessageListProps) {
  const [scrollElement, setScrollElement] = useState<HTMLDivElement | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);
  const [pinnedToBottom, setPinnedToBottom] = useState(true);
  const showReasoning = useShowReasoning();

  // The transcript is built in two halves so a streamed frame pays for one of
  // them at most (spec: chat-streaming-performance). Above the run boundary
  // nothing can change while a run streams, so those items are built once per
  // conversation snapshot; below it, only the run's own turns rebuild when its
  // reasoning grows.
  const runStart = useMemo(() => runStartIndex(messages), [messages]);
  const settledItems = useMemo(() => buildDisplayItems(messages, 0, runStart, [], false), [messages, runStart]);
  const runReasoning = showReasoning ? reasoning : NO_SEGMENTS;
  const runItems = useMemo(() => {
    const built = buildDisplayItems(messages, runStart, messages.length, runReasoning, reasoningTrimmed);
    return suppressCurrentRunActivity ? built.filter((item) => item.kind !== "activity") : built;
  }, [messages, runStart, runReasoning, reasoningTrimmed, suppressCurrentRunActivity]);
  const items = useMemo(() => [...settledItems, ...runItems], [settledItems, runItems]);

  // Keep the newest run outside the virtual window for its whole visual
  // lifetime. During generation that location is the live `tail`; after the
  // durable snapshot arrives the same location draws `runItems`. Moving the
  // finished answer into a newly measured virtual row changed the list height
  // and remounted its markdown at the terminal event, which produced the
  // end-of-response full-text flash. Older turns enter virtualization only
  // when the next user turn creates a new run boundary.
  const virtualItems = settledItems;
  const durableRunItems = tailHasLatestAnswer ? [] : runItems;
  const rowCount = virtualItems.length;

  // The run's final answer is drawn by ONE element, `CurrentAnswer`, whichever
  // source holds it: the live text while the run is in flight, the saved row once
  // it lands. Drawing it as a live row and then as a different saved row rebuilt
  // the answer at the handover, which read as the response being generated twice.
  const answerItem = useMemo(() => {
    for (let index = runItems.length - 1; index >= 0; index -= 1) {
      const item = runItems[index];
      if (item.kind === "message" && item.message.role === "assistant" && item.isLastProse) return item;
    }
    return null;
  }, [runItems]);

  // Entrance discipline (spec: chat-motion-system): only ids APPENDED after
  // mount animate. A bulk swap (conversation open) or any id vanishing seeds
  // silently — that's exactly the no-replay / no-double-paint contract.
  const prevIdsRef = useRef<Set<string> | null>(null);
  const enteringIdsRef = useRef(new Set<string>());
  const previousTailHadAnswerRef = useRef(tailHasLatestAnswer);
  useMemo(() => {
    const durableAnswerIsReplacingLiveTail = previousTailHadAnswerRef.current && !tailHasLatestAnswer;
    previousTailHadAnswerRef.current = tailHasLatestAnswer;
    const currentIds = new Set(items.map((item) => item.id));
    const previous = prevIdsRef.current;
    prevIdsRef.current = currentIds;
    if (previous === null) return;
    let vanished = false;
    for (const id of previous) {
      if (!currentIds.has(id)) {
        vanished = true;
        break;
      }
    }
    const fresh: string[] = [];
    for (const id of currentIds) {
      if (!previous.has(id)) fresh.push(id);
    }
    // The same answer just changed source, from the live text to its saved row.
    // Treating that row as newly arrived fades/translates the already-visible
    // answer back onto the page, which looks like a full chat refresh at the end
    // of every response.
    if (durableAnswerIsReplacingLiveTail || vanished || fresh.length === 0 || fresh.length > 2) return;
    for (const id of fresh) enteringIdsRef.current.add(id);
    // setTimeout (not setInterval — frontend privilege boundary) retires the
    // class after it played, so later re-mounts render motionless.
    window.setTimeout(() => {
      for (const id of fresh) enteringIdsRef.current.delete(id);
    }, ENTER_REPLAY_GUARD_MS);
  }, [items, tailHasLatestAnswer]);

  // A new key callback invalidates every cached measurement, even when the
  // history is unchanged. Streamed tail updates must not re-estimate old rows.
  const getScrollElement = useCallback(() => scrollElement, [scrollElement]);
  const estimateSize = useCallback((index: number) => estimateItemHeight(virtualItems[index]), [virtualItems]);
  const getItemKey = useCallback((index: number) => virtualItems[index]?.id ?? index, [virtualItems]);
  const virtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement,
    estimateSize,
    getItemKey,
    overscan: 8,
    measureElement: measureMessageRow,
    useFlushSync: false,
    observeElementRect: observeScrollRect,
  });
  const virtualHeight = virtualizer.getTotalSize();

  // The scroll element, not the last virtual row: the run in flight renders in
  // the tail AFTER the virtual window, so the bottom of the flow is past the
  // last row the virtualizer knows about.
  const scrollToBottom = useCallback(() => {
    const element = scrollElement;
    if (!element) return;
    const reduceMotion =
      typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    element.scrollTo({ top: element.scrollHeight, behavior: reduceMotion ? "auto" : "smooth" });
    pinnedRef.current = true;
    setPinnedToBottom(true);
  }, [scrollElement]);

  const pinnedRef = useRef(true);
  const lastScrollTopRef = useRef(0);
  const followBottom = useCallback(() => {
    if (!scrollElement) return;
    scrollElement.scrollTop = scrollElement.scrollHeight;
    lastScrollTopRef.current = scrollElement.scrollTop;
  }, [scrollElement]);

  // Reset before paint, including the commit that attaches the scroll element.
  // The newest run is outside the virtual window: aligning its last row would
  // stop at the user message and hide the answer below it.
  useLayoutEffect(() => {
    pinnedRef.current = true;
    setPinnedToBottom(true);
    followBottom();
  }, [conversationId, followBottom]);

  // A conversation may load after its identity changes. Keep the full flow
  // pinned as its snapshot arrives; ResizeObserver handles later measurements.
  useLayoutEffect(() => {
    if (pinnedRef.current) followBottom();
  }, [followBottom, items, virtualHeight]);

  useEffect(() => {
    if (!scrollElement) return;
    const onScroll = () => {
      const distance = scrollElement.scrollHeight - scrollElement.scrollTop - scrollElement.clientHeight;
      // A delayed event from our last bottom correction can arrive after the
      // next streamed layout has grown. Distance alone would detach the reader
      // even though they never scrolled. Only upward movement detaches; reaching
      // the bottom reattaches, including a scrollbar drag or keyboard scroll.
      if (distance <= NEAR_BOTTOM_PX) pinnedRef.current = true;
      else if (scrollElement.scrollTop < lastScrollTopRef.current) pinnedRef.current = false;
      lastScrollTopRef.current = scrollElement.scrollTop;
      setPinnedToBottom(pinnedRef.current);
    };
    scrollElement.addEventListener("scroll", onScroll, { passive: true });
    onScroll();
    return () => scrollElement.removeEventListener("scroll", onScroll);
  }, [scrollElement]);

  // Web's stick-to-bottom model (`useStickToBottom`): a streamed delta grows
  // the CURRENT row — neither the message count nor the last id changes — so
  // following the stream has to key off content GROWTH. The observed wrapper
  // spans the virtualizer's total size AND the live tail, so one
  // ResizeObserver fires once per layout pass (not per token) and re-pins the
  // bottom — but only while the reader is already near it. Someone who
  // scrolled up to read is never yanked back down; they get the
  // "New response" chip instead.
  useEffect(() => {
    if (!scrollElement) return;
    if (typeof ResizeObserver === "undefined") return;
    const list = listRef.current;
    if (!list) return;
    const observer = new ResizeObserver(() => {
      if (!pinnedRef.current) return;
      followBottom();
    });
    observer.observe(list);
    return () => observer.disconnect();
  }, [scrollElement, followBottom]);

  // A disclosure reports its settled layout even when reduced motion skips
  // animation. Route that signal through the same measurement owner as the
  // row's ResizeObserver, so the two cannot write competing height estimates.
  useEffect(() => {
    if (!scrollElement) return;
    const onRemeasure = (event: Event) => {
      const target = event.target;
      if (!(target instanceof HTMLElement)) return;
      const row = target.closest<HTMLElement>("[data-index]");
      if (!row) return;
      const index = Number(row.dataset["index"]);
      if (!Number.isFinite(index)) return;
      virtualizer.measureElement(row);
    };
    scrollElement.addEventListener(REMEASURE_EVENT, onRemeasure);
    return () => scrollElement.removeEventListener(REMEASURE_EVENT, onRemeasure);
  }, [scrollElement, virtualizer]);

  // A width change rewraps prose and path chips. Refresh mounted rows through
  // the same measurement path; offscreen rows refresh when they mount again.
  useEffect(() => {
    if (!scrollElement) return;
    if (typeof ResizeObserver === "undefined") return;
    // The first observation is a baseline, not a resize: acting on it would
    // re-measure every row on mount for no change.
    let lastWidth: number | null = null;
    const observer = new ResizeObserver((entries) => {
      const width = entries[entries.length - 1]?.contentRect.width;
      if (width === undefined || width === lastWidth) return;
      const settled = lastWidth !== null;
      lastWidth = width;
      if (!settled) return;
      for (const row of scrollElement.querySelectorAll<HTMLElement>("[data-index]")) {
        const index = Number(row.dataset["index"]);
        if (Number.isFinite(index)) virtualizer.measureElement(row);
      }
    });
    observer.observe(scrollElement);
    return () => observer.disconnect();
  }, [scrollElement, virtualizer]);

  // Two states of one control, not two controls. Scrolled away from the bottom
  // always offers the way back (xfalcon-web's `ScrollToBottom`); when something
  // arrived while the reader was up there, the same button says so — that is
  // information a bare arrow would throw away.
  const scrolledAway = !pinnedToBottom && rowCount > 0;
  const showNewResponse = scrolledAway && Boolean(followStream);

  // The brand-accent rail marks exactly one turn — the last assistant one
  // (spec: chat-thread-layout). Scanned over the durable items only; while a
  // run is in flight its streaming row is in the tail and carries the rail
  // directly.
  const latestDurableAssistantId = useMemo(() => {
    for (let index = items.length - 1; index >= 0; index -= 1) {
      const item = items[index];
      if (item?.kind === "message" && item.message.role === "assistant") return item.id;
    }
    return null;
  }, [items]);

  const renderItem = (item: DisplayItem, className?: string) => (
    <div className={className}>
      {item.kind === "activity" ? (
        <div className="py-1.5">
          <DurableToolTrail
            asides={
              item.reasoning
                ? [
                    {
                      kind: "reasoning",
                      text: item.reasoning.text,
                      afterCallCount: 0,
                      trimmed: item.reasoning.trimmed,
                    },
                  ]
                : undefined
            }
            calls={item.calls}
          />
          {/* A saved report the turn asked to show, drawn where web's viewer
            appears: under the activity that opened it. */}
          <ChatReportViews calls={item.calls} />
        </div>
      ) : (
        <MessageRow
          conversationId={conversationId}
          isLatestAssistant={!tailHasLatestAnswer && item.id === latestDurableAssistantId}
          isLastProse={item.isLastProse}
          message={item.message}
          showRibbon={item.opensResponse}
          onEdit={onEdit}
          onRegenerate={onRegenerate}
          revisionPending={revisionPending}
        />
      )}
    </div>
  );

  // The current run's blocks, in order, with the answer under its one key. Keyed
  // children keep the answer's element when it changes source or position.
  const currentRunBlocks: ReactNode[] = [];
  let answerPlaced = false;
  const answerNode = (index: number, saved: SavedAnswer | null) => (
    <CurrentAnswer
      className={saved !== null && enteringIdsRef.current.has(answerItem?.id ?? "") ? "message-enter" : undefined}
      conversationId={conversationId}
      index={index}
      key={CURRENT_ANSWER_KEY}
      live={liveAnswer}
      revisionPending={revisionPending}
      saved={saved}
      showRibbon={liveAnswer !== null ? !responseAlreadyOpen(messages) : (answerItem?.opensResponse ?? true)}
      onEdit={onEdit}
      onRegenerate={onRegenerate}
    />
  );
  durableRunItems.forEach((item, index) => {
    if (item === answerItem) {
      answerPlaced = true;
      currentRunBlocks.push(answerNode(index, { message: item.message, isLastProse: item.isLastProse }));
      return;
    }
    currentRunBlocks.push(
      <div data-current-run-index={index} key={item.id}>
        {renderItem(item, enteringIdsRef.current.has(item.id) ? "message-enter" : undefined)}
      </div>,
    );
  });
  if (!answerPlaced && liveAnswer !== null) currentRunBlocks.push(answerNode(currentRunBlocks.length, null));

  return (
    <div className="relative min-h-0 flex-1">
      <div
        ref={setScrollElement}
        className="h-full min-h-0 overflow-y-auto py-4"
        style={{ minHeight: 240, overflowAnchor: pinnedToBottom ? "none" : "auto" }}
      >
        <div ref={listRef} className="chat-column-messages">
          <ul className="relative list-none p-0" style={{ height: `${virtualHeight}px` }}>
            {virtualizer.getVirtualItems().map((virtualItem) => {
              const item = virtualItems[virtualItem.index];
              if (item === undefined) return null;
              return (
                <li
                  key={item.id}
                  className="absolute left-0 top-0 w-full"
                  style={{ transform: `translateY(${virtualItem.start}px)` }}
                >
                  <div ref={virtualizer.measureElement} data-index={virtualItem.index}>
                    {renderItem(item, enteringIdsRef.current.has(item.id) ? "message-enter" : undefined)}
                  </div>
                </li>
              );
            })}
          </ul>
          {tail}
          {currentRunBlocks}
          {footer}
        </div>
      </div>
      {scrolledAway ? jump(showNewResponse, scrollToBottom) : null}
    </div>
  );
}
