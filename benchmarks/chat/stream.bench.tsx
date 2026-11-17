import React, { Profiler, memo } from "react";
import { act, cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ChatPane } from "../../src/features/chat/ChatPane";
import { LiveRunTimeline } from "../../src/features/chat/LiveRunTimeline";
import { MessageList } from "../../src/features/chat/MessageList";
import { MessageRow } from "../../src/features/chat/MessageRow";
import {
  applyConversationView,
  applyInferenceForTests,
  getChatSnapshot,
  resetChatStore,
} from "../../src/features/chat/chatStore";
import { clearTurnEdit, startTurnEdit } from "../../src/features/chat/turnEditStore";
import type { ChatMessageRow } from "../../src/features/chat/chatStore";
import type { ChatConversationView, ToolInvocationEvent } from "../../src/lib/generated/contracts";

/**
 * Streaming-render baseline for the transcript.
 *
 * The metric that matters is STRUCTURAL, not wall-clock: how many times a
 * settled fence's card re-renders while later tokens arrive. jsdom timings are
 * not browser-accurate, but a card that renders once per streamed frame is
 * re-parsing markdown it already parsed, and that is a defect in any runtime.
 *
 * Mirrors LibreChat's `MarkdownBlocks.bench.tsx` methodology (references/
 * LibreChat): stub the expensive leaf, count its renders across a fixed number
 * of stream steps, and report the Profiler's summed `actualDuration` alongside.
 */

const cardRenders = { code: 0, tools: 0 };

vi.mock("../../src/features/chat/richContent/cards", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../src/features/chat/richContent/cards")>();
  return {
    ...actual,
    CodeArtifact: function CountingCodeArtifact() {
      cardRenders.code += 1;
      return null;
    },
  };
});

// The REAL live card, wrapped in a counter. A settled round's card must not
// re-render while a later answer streams: its `events` slice used to be rebuilt
// by `activity.filter` inside a memo keyed on the streaming text, which
// re-derived and re-measured every row of it once per token.
vi.mock("../../src/features/chat/ToolActivityCard", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../src/features/chat/ToolActivityCard")>();
  const Real = actual.LiveRunActivity;
  return {
    ...actual,
    // MEMOIZED, like the component it stands in for. A bare function wrapper
    // would re-render whenever its parent did and count that, measuring the
    // wrapper instead of the card.
    LiveRunActivity: memo(function CountingLiveRunActivity(props: Parameters<typeof Real>[0]) {
      cardRenders.tools += 1;
      return <Real {...props} />;
    }),
  };
});

// The REAL composer, counted on every render. It sits below the transcript in
// the chat column, so a parent that re-renders per streamed delta re-renders it
// too, whether or not any of its own inputs changed.
const composerRenders = { count: 0 };

vi.mock("../../src/features/chat/Composer", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../src/features/chat/Composer")>();
  const Real = actual.Composer;
  return {
    ...actual,
    Composer: function CountingComposer(props: Parameters<typeof Real>[0]) {
      composerRenders.count += 1;
      return <Real {...props} />;
    },
  };
});

const LANGS = ["python", "sql", "typescript", "bash", "json", "dax"];

/** A realistic assistant answer: prose, fenced code, and a table. */
function buildAnswer(sections: number): string {
  const parts: string[] = [];
  for (let i = 0; i < sections; i += 1) {
    parts.push(`## Section ${i + 1}`, "");
    parts.push(
      `Paragraph ${i + 1} explains the query below with some **bold** and \`inline\` ` +
        `text, long enough to cost realistic reflow while the answer streams.`,
      "",
    );
    parts.push("```" + LANGS[i % LANGS.length]);
    for (let line = 0; line < 8; line += 1) {
      parts.push(`select col_${i}_${line} from table_${i} where id = ${line};`);
    }
    parts.push("```", "");
    if (i % 3 === 0) {
      parts.push("| Name | Type | Value |", "| --- | --- | --- |");
      for (let row = 0; row < 5; row += 1) {
        parts.push(`| item_${i}_${row} | number | ${row * i} |`);
      }
      parts.push("");
    }
  }
  return parts.join("\n");
}

function prefixes(content: string, steps: number): string[] {
  const out: string[] = [];
  for (let step = 1; step <= steps; step += 1) {
    out.push(content.slice(0, Math.ceil((content.length * step) / steps)));
  }
  return out;
}

function priorTurns(count: number): ChatMessageRow[] {
  const rows: ChatMessageRow[] = [];
  for (let i = 0; i < count; i += 1) {
    rows.push({
      id: `m-${i}`,
      role: i % 2 === 0 ? "user" : "assistant",
      text: `Turn ${i} of the transcript, with enough prose to wrap onto a second line.`,
      sequence: BigInt(i + 1),
      toolCalls: [],
    });
  }
  return rows;
}

const STEPS = 80;
const SECTIONS = 12;

afterEach(cleanup);

beforeEach(() => {
  cardRenders.code = 0;
  cardRenders.tools = 0;
  composerRenders.count = 0;
  // The benchmark counts renders per streamed frame, so each frame's text must
  // reach the row at once. The typewriter reveal is a separate, time-based layer.
  vi.stubGlobal(
    "matchMedia",
    vi.fn(() => ({ matches: true })),
  );
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
    width: 640,
    height: 480,
    top: 0,
    left: 0,
    bottom: 480,
    right: 640,
    x: 0,
    y: 0,
    toJSON: () => ({}),
  });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => 480 });
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

/**
 * Streams the real answer through the live answer row, counting fence-card renders.
 *
 * `prior` is passed by the SAME reference on every frame, which is what the real
 * caller does: the in-flight answer lives outside the virtual window, and the
 * durable transcript holds its identity for the length of the stream.
 */
function runStream(priorCount: number) {
  const answer = buildAnswer(SECTIONS);
  const frames = prefixes(answer, STEPS);
  const prior = priorTurns(priorCount);

  let totalMs = 0;
  let commits = 0;
  const onRender = (_id: string, _phase: string, actualDuration: number) => {
    totalMs += actualDuration;
    commits += 1;
  };

  const tree = (text: string) => (
    <Profiler id="transcript" onRender={onRender}>
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList
          conversationId="conv-bench"
          liveAnswer={{ text, streaming: true }}
          messages={prior}
          tailHasLatestAnswer
        />
      </div>
    </Profiler>
  );

  const started = performance.now();
  const { rerender } = render(tree(frames[0]));
  for (let step = 1; step < frames.length; step += 1) {
    act(() => {
      rerender(tree(frames[step]));
    });
  }
  const wallMs = performance.now() - started;

  return { answerBytes: answer.length, codeRenders: cardRenders.code, totalMs, commits, wallMs };
}

/**
 * Streams a tail whose content is trivial, so what is measured is the
 * TRANSCRIPT's own per-frame cost and not the growing answer's markdown.
 *
 * This is the isolation the old bench got for free by putting the in-flight row
 * outside the virtual window — a case that cannot happen now the row is in the
 * tail, and never happened in the app either: the answer being written is
 * exactly where the reader is looking.
 */
function runTranscriptOverhead(priorCount: number) {
  const frames = prefixes(buildAnswer(SECTIONS), STEPS);
  const prior = priorTurns(priorCount);

  let totalMs = 0;
  let commits = 0;
  const onRender = (_id: string, _phase: string, actualDuration: number) => {
    totalMs += actualDuration;
    commits += 1;
  };

  const tree = (text: string) => (
    <Profiler id="transcript" onRender={onRender}>
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList messages={prior} tail={<div>{text.length}</div>} />
      </div>
    </Profiler>
  );

  const { rerender } = render(tree(frames[0]));
  for (let step = 1; step < frames.length; step += 1) {
    act(() => {
      rerender(tree(frames[step]));
    });
  }
  return { totalMs, commits };
}

/**
 * Budgets, not observations.
 *
 * The structural count carries the gate: how many times a settled fence's card
 * renders across a fixed stream is deterministic, and it is what regressed when
 * the whole message re-parsed per token. The timings are recorded beside it, at
 * a ceiling loose enough that a slow machine does not fail the build and tight
 * enough that a return to whole-message rendering does — a jsdom millisecond is
 * a relative signal, never a product service level
 * (`docs/contributing.md`).
 */

/**
 * Fence-card renders allowed across 80 steps of a 12-fence answer.
 *
 * 12 is the floor: every fence renders once when its block completes. The
 * measured figure is 60 — the last, still-growing block re-renders whatever
 * fence it holds. Before per-block memoization this was 500, one render of every
 * settled fence on every frame.
 */
const MAX_FENCE_RENDERS = 120;
/**
 * What the transcript itself costs a stream, below 500 settled turns, with the
 * answer's own markdown excluded.
 *
 * The ONE timing that discriminates. The transcript contributes this while
 * drawing nothing new, so it must not scale with the conversation: rebuilding
 * every display item per frame measured ~910 ms here, and re-keying the virtual
 * window per frame added to it. Neither fits under this ceiling.
 */
const MAX_TRANSCRIPT_MS_500 = 750;
/**
 * Renders of a settled round's tool card across 80 steps of a streaming answer.
 *
 * The measured count is 1: the card renders once, on mount, and is not
 * re-rendered as the later answer streams. 8 is the ceiling this gate allows,
 * not the expected figure. A count near 80 means the card is being re-derived
 * per token, which is what a memo keyed on the streaming text did.
 */
const MAX_TOOL_CARD_RENDERS = 8;

describe("chat streaming render budget", () => {
  it("re-renders only the growing block, not every settled fence", () => {
    const result = runStream(0);
    console.log(
      `[stream/empty] answer=${result.answerBytes}B steps=${STEPS} fences=${SECTIONS} ` +
        `codeRenders=${result.codeRenders} commits=${result.commits} ` +
        `profilerMs=${result.totalMs.toFixed(0)} wallMs=${result.wallMs.toFixed(0)}`,
    );
    expect(result.codeRenders).toBeGreaterThan(0);
    expect(result.codeRenders).toBeLessThan(MAX_FENCE_RENDERS);
  });

  it("keeps the same fence-render count below a long transcript", () => {
    const result = runStream(500);
    console.log(
      `[stream/500] answer=${result.answerBytes}B steps=${STEPS} fences=${SECTIONS} ` +
        `codeRenders=${result.codeRenders} commits=${result.commits} ` +
        `profilerMs=${result.totalMs.toFixed(0)} wallMs=${result.wallMs.toFixed(0)}`,
    );
    // The count is a property of the ANSWER, not of what is above it. A
    // conversation-dependent figure here would mean the transcript is re-parsing
    // the answer on the transcript's behalf.
    expect(result.codeRenders).toBeLessThan(MAX_FENCE_RENDERS);
  });

  it("does not re-render a settled round's tool card per token", () => {
    const frames = prefixes(buildAnswer(SECTIONS), STEPS);
    const activity = [
      {
        type: "requested",
        version: 1,
        invocation_id: "inv-1",
        run_id: "bench",
        tool_id: "mcp.tenant.ida_query",
      },
      { type: "completed", version: 1, invocation_id: "inv-1", truncated: false },
    ] as unknown as ToolInvocationEvent[];

    // Every array prop is hoisted, because that is what the real caller hands
    // over: `activity` and `reasoning` are store-owned references that change
    // only when a tool event or a fragment arrives. A fresh literal
    // per frame would re-key the timeline's settled-block memo and measure the
    // fixture rather than the component.
    const prior = priorTurns(4);
    const noReasoning: never[] = [];
    const tree = (text: string) => (
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList
          conversationId="conv-bench"
          liveAnswer={{ text, streaming: true }}
          messages={prior}
          tail={<LiveRunTimeline activity={activity} reasoning={noReasoning} reasoningTrimmed={false} runActive />}
          tailHasLatestAnswer
        />
      </div>
    );

    const { rerender } = render(tree(frames[0]));
    for (let step = 1; step < frames.length; step += 1) {
      act(() => {
        rerender(tree(frames[step]));
      });
    }
    console.log(`[toolCard] steps=${STEPS} renders=${cardRenders.tools}`);
    expect(cardRenders.tools).toBeGreaterThan(0);
    expect(cardRenders.tools).toBeLessThanOrEqual(MAX_TOOL_CARD_RENDERS);
  });

  it("costs almost nothing for the transcript to sit above a stream", () => {
    const empty = runTranscriptOverhead(0);
    cleanup();
    const long = runTranscriptOverhead(500);
    console.log(
      `[overhead] steps=${STEPS} emptyMs=${empty.totalMs.toFixed(0)} ` +
        `longMs=${long.totalMs.toFixed(0)} commits=${long.commits}`,
    );
    expect(long.commits).toBeGreaterThan(0);
    expect(long.totalMs).toBeLessThan(MAX_TRANSCRIPT_MS_500);
  });
});

/**
 * Composer renders across 80 deltas of a live answer, through the real
 * `ChatPane` and the real chat store.
 *
 * The composer's inputs do not change while text streams, so the budget is a
 * small constant: the commit that flips the run to streaming, and nothing per
 * delta. The counter is reset after the initial render, so only renders during
 * the delta loop are counted, the first delta's included. Before `ComposerDock`
 * was memoized this was one render per delta.
 */
const MAX_COMPOSER_RENDERS = 2;

describe("composer during a streamed answer", () => {
  let frameQueue: FrameRequestCallback[] = [];

  beforeEach(() => {
    frameQueue = [];
    // Coalesced store commits wait for a frame. The bench runs the frame on
    // demand, so each delta is one commit and the count is deterministic.
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      frameQueue.push(callback);
      return frameQueue.length;
    });
  });

  afterEach(() => {
    resetChatStore();
  });

  it("does not re-render the composer for each streamed delta", () => {
    const answer = buildAnswer(SECTIONS);
    const runId = "run-bench";
    const conversation: ChatConversationView = {
      version: 1,
      conversation_id: "conv-bench",
      title: "Streaming bench",
      availability: "live",
      messages: [
        {
          version: 1,
          message_id: "u-1",
          role: "user",
          text: "Explain the query",
          sequence: BigInt(1),
          tool_calls: [],
          attachments: [],
        },
      ],
      active_run_id: runId,
    };
    // Hoisted, as `AppShell` hands them over: a fresh literal per delta would
    // measure the bench's fixture instead of the composer's memo.
    const pane = {
      activeProjectName: null,
      disabled: false,
      onClearProject: () => {},
      onDismissNotice: () => {},
      onOpenSettings: () => {},
      onResetConversation: () => {},
      onRetryRun: () => {},
      onStop: () => {},
      onSubmit: () => {},
      projectUnavailable: false,
      stopping: false,
      viewerName: "Tester",
      mcpServers: ["tenant.ida"],
      projectFiles: [],
      skills: [],
    };

    applyConversationView(conversation);
    render(<ChatPane {...pane} />);
    composerRenders.count = 0;

    for (let step = 0; step < STEPS; step += 1) {
      const from = Math.floor((answer.length * step) / STEPS);
      const to = Math.floor((answer.length * (step + 1)) / STEPS);
      act(() => {
        applyInferenceForTests({
          type: "text_delta",
          version: 1,
          run_id: runId,
          sequence: step,
          text: answer.slice(from, to),
        });
        const pending = frameQueue;
        frameQueue = [];
        for (const callback of pending) callback(0);
      });
    }

    // Every delta reached the store, so the count measures a stream that landed.
    expect(getChatSnapshot().streamingText).toBe(answer);
    console.log(`[composer] steps=${STEPS} composerRenders=${composerRenders.count}`);
    expect(composerRenders.count).toBeLessThanOrEqual(MAX_COMPOSER_RENDERS);
  });
});

/**
 * Settled rows re-rendered per keystroke of a turn edit.
 *
 * The editor reads the draft; a row reads only which message is being edited.
 * Typing must therefore re-render the editing row and nothing else. Forty rows
 * is a conversation's worth of mounted history, not the virtual window.
 */
describe("turn edit keystrokes", () => {
  afterEach(() => {
    clearTurnEdit();
  });

  it("re-renders the editing row and no other row per keystroke", async () => {
    const messages = priorTurns(40);
    const rowRenders = new Map<string, number>();
    const onEdit = () => {};
    render(
      <div className="flex flex-col">
        {messages.map((message) => (
          <Profiler
            key={message.id}
            id={message.id}
            onRender={() => rowRenders.set(message.id, (rowRenders.get(message.id) ?? 0) + 1)}
          >
            <MessageRow message={message} onEdit={onEdit} />
          </Profiler>
        ))}
      </div>,
    );

    act(() => startTurnEdit("m-0", messages[0].text));
    rowRenders.clear();

    const typed = "abcdefghijklmnopqrst";
    const textarea = screen.getByRole("textbox", { name: "Edit message" });
    await userEvent.setup().type(textarea, typed);
    expect(textarea).toHaveValue(messages[0].text + typed);

    const editingRow = rowRenders.get("m-0") ?? 0;
    const otherRows = [...rowRenders].filter(([id]) => id !== "m-0");
    const otherRowRenders = otherRows.reduce((sum, [, count]) => sum + count, 0);
    console.log(
      `[turnEdit] keystrokes=${typed.length} editingRowRenders=${editingRow} ` + `otherRowRenders=${otherRowRenders}`,
    );
    expect(editingRow).toBeGreaterThan(0);
    expect(otherRowRenders).toBe(0);
  });
});
