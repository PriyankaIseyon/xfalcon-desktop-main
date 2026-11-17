import { describe, expect, it } from "vitest";
import type { ToolInvocationEvent, ToolStepSummaryV1 } from "../../lib/generated/contracts";
import {
  formatElapsed,
  formatPayload,
  parseToolId,
  shortenStepPath,
  stateForDurableCall,
  stateForEvent,
  stepRow,
} from "./toolPresentation";

describe("parseToolId", () => {
  it("splits mcp.<server>.<tool> into title and server hint", () => {
    expect(parseToolId("mcp.tenant-ida.ida_query")).toEqual({ title: "ida_query", serverHint: "tenant-ida" });
  });

  it("keeps tool names containing dots intact after the server segment", () => {
    expect(parseToolId("mcp.srv.name.with.dots")).toEqual({ title: "name.with.dots", serverHint: "srv" });
  });

  it("renders non-mcp ids verbatim", () => {
    expect(parseToolId("builtin.search")).toEqual({ title: "builtin.search", serverHint: null });
  });

  it("does not guess on malformed mcp ids", () => {
    expect(parseToolId("mcp.only-server")).toEqual({ title: "mcp.only-server", serverHint: null });
  });
});

describe("stateForEvent", () => {
  const event = (partial: Partial<ToolInvocationEvent> & { type: ToolInvocationEvent["type"] }) =>
    ({ invocation_id: "i-1", run_id: "r-1", ...partial }) as ToolInvocationEvent;

  it("maps the backend lifecycle onto presentation states", () => {
    expect(stateForEvent(event({ type: "requested", tool_id: "t" } as never))).toBe("input-streaming");
    expect(stateForEvent(event({ type: "approval_required", tool_id: "t" } as never))).toBe("approval-requested");
    expect(stateForEvent(event({ type: "approved" } as never))).toBe("input-streaming");
    expect(stateForEvent(event({ type: "started" } as never))).toBe("input-available");
    expect(stateForEvent(event({ type: "progress", sequence: 1, summary: "" } as never))).toBe("input-available");
    expect(stateForEvent(event({ type: "completed", truncated: false } as never))).toBe("output-available");
    expect(stateForEvent(event({ type: "failed", message: "boom", retryable: false } as never))).toBe("output-error");
    expect(stateForEvent(event({ type: "denied", reason_code: "nope" } as never))).toBe("output-denied");
    expect(stateForEvent(event({ type: "cancelled" } as never))).toBe("cancelled");
  });
});

describe("stateForDurableCall", () => {
  const call = (status: "answered" | "unanswered") => ({
    call_id: "c",
    name: "n",
    status,
    arguments: null,
    arguments_truncated: false,
    result: null,
    result_truncated: false,
    step: null,
  });

  it("maps answered/unanswered onto success/no-result", () => {
    expect(stateForDurableCall(call("answered"))).toBe("output-available");
    expect(stateForDurableCall(call("unanswered"))).toBe("no-result");
  });
});

describe("formatPayload", () => {
  it("pretty-prints valid JSON", () => {
    expect(formatPayload('{"a":1}')).toBe('{\n  "a": 1\n}');
  });

  it("returns non-JSON verbatim and passes null through", () => {
    expect(formatPayload("plain text")).toBe("plain text");
    expect(formatPayload(null)).toBeNull();
    expect(formatPayload(undefined)).toBeNull();
  });

  it("skips pretty-printing past the size threshold", () => {
    const big = `"${"x".repeat(25_000)}"`;
    expect(formatPayload(big)).toBe(big);
  });
});

describe("formatElapsed", () => {
  it("formats sub-second, seconds, and minutes", () => {
    expect(formatElapsed(400)).toBe("<1s");
    expect(formatElapsed(2400)).toBe("2s");
    expect(formatElapsed(61_000)).toBe("1m 1s");
    expect(formatElapsed(120_000)).toBe("2m");
  });
});

describe("stepRow (spec: chat-tool-trail)", () => {
  const summary = (paths: string[], pathTotal: number): ToolStepSummaryV1 => ({
    version: 1,
    label: "Listed 400 entries",
    paths: paths.map((path) => ({ path, is_sensitive: false })),
    path_total: pathTotal,
  });

  it("has nothing to draw for a call the projection did not summarize", () => {
    expect(stepRow(null)).toBeNull();
    expect(stepRow(undefined)).toBeNull();
  });

  it("passes the projection's label through untouched", () => {
    // Composing a second label here would drift from the first, and only one of
    // the two can be checked against the outcome it describes.
    expect(stepRow(summary(["a.ts"], 1))?.label).toBe("Listed 400 entries");
  });

  it("states both numbers when it is showing fewer paths than the run touched", () => {
    expect(stepRow(summary(["a.ts", "b.ts"], 400))?.boundNote).toBe("2 of 400 paths");
  });

  it("says nothing about a bound when nothing was cut", () => {
    expect(stepRow(summary(["a.ts", "b.ts", "c.ts"], 3))?.boundNote).toBeNull();
  });

  it("says nothing about a bound when there is no visible list to qualify", () => {
    // A count-only step (a patch plan, a listing whose directory was unusable)
    // carries its number in the label; "0 of 3" would be a placeholder standing
    // in for the difference.
    expect(stepRow(summary([], 3))?.boundNote).toBeNull();
  });

  it("carries the classifier's verdict onto the path it belongs to", () => {
    const row = stepRow({
      version: 1,
      label: "Listed 2 entries",
      paths: [
        { path: "src/cards.tsx", is_sensitive: false },
        { path: ".env", is_sensitive: true },
      ],
      path_total: 2,
    });
    expect(row?.paths.map((path) => path.sensitive)).toEqual([false, true]);
  });
});

describe("shortenStepPath", () => {
  it("leaves a path that already fits alone", () => {
    expect(shortenStepPath("src/features/chat/MessageList.tsx", 44)).toBe("src/features/chat/MessageList.tsx");
  });

  it("keeps the file name, which is what a reader identifies a path by", () => {
    const long = `src/features/${"nested/".repeat(8)}MessageList.tsx`;
    const shortened = shortenStepPath(long, 44);
    expect(shortened).toHaveLength(44);
    expect(shortened.endsWith("MessageList.tsx")).toBe(true);
    expect(shortened).toContain("…");
  });
});
