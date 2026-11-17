import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({
  invoke,
  convertFileSrc: (id: string) => `http://xf-preview.localhost/${id}`,
}));

import { captureMissingPreviewOnce, captureReportPreview, resetPreviewCaptureAttempts } from "./reportPreviewCapture";
import { applyReportList, getReportsSnapshot, resetReportsStore } from "./reportsStore";

const ID = "r1";
const page = { version: 1, id: ID, document_id: "doc-capture", nonce: "nonce-1" };
const saved = { id: ID, thumbnail_updated_at_ms: 7n } as const;

/** The capture frame the module puts on the page, once it has. */
function captureFrame(): HTMLIFrameElement | null {
  return document.querySelector<HTMLIFrameElement>('iframe[title="Report preview capture"]');
}

/** Plays the page's side: load, take the reply port, and answer on it. */
async function pageAnswers(reply: (nonce: string) => unknown) {
  await vi.waitFor(() => expect(captureFrame()).not.toBeNull());
  const frame = captureFrame()!;
  const post = vi.spyOn(frame.contentWindow!, "postMessage");
  frame.dispatchEvent(new Event("load"));
  expect(post).toHaveBeenCalledTimes(1);
  const [message, target, transfer] = post.mock.calls[0] as unknown as [
    { type: string; nonce: string },
    string,
    MessagePort[],
  ];
  expect(message).toEqual({ type: "ida-thumbnail-connect", nonce: page.nonce });
  expect(target).toBe("*");
  transfer[0].postMessage(reply(message.nonce));
  return frame;
}

function answer(byCommand: Record<string, (args: Record<string, unknown>) => unknown>) {
  invoke.mockImplementation((command: string, args: Record<string, unknown>) => {
    const reply = byCommand[command];
    return reply === undefined
      ? Promise.reject(new Error(`unexpected ${command}`))
      : Promise.resolve().then(() => reply(args));
  });
}

beforeEach(() => {
  invoke.mockReset();
  resetReportsStore();
  resetPreviewCaptureAttempts();
});
afterEach(() => {
  vi.useRealTimers();
  captureFrame()?.remove();
});

describe("captureReportPreview", () => {
  it("renders the capture page in a hidden sandboxed frame and stores the one image it sends", async () => {
    answer({ report_thumbnail_capture_open: () => page, report_thumbnail_save: () => saved });
    applyReportList([{ id: ID, thumbnail_updated_at_ms: null } as never]);

    const capture = captureReportPreview(ID);
    const frame = await pageAnswers((nonce) => ({ type: "ida-thumbnail", nonce, image_base64: "iVBORw==" }));

    await expect(capture).resolves.toEqual(saved);
    expect(frame).toHaveAttribute("sandbox", "allow-scripts");
    expect(frame).toHaveAttribute("src", "http://xf-preview.localhost/doc-capture");
    expect(frame).toHaveAttribute("aria-hidden", "true");
    expect(frame.style.opacity).toBe("0");
    expect(frame.style.pointerEvents).toBe("none");
    expect(invoke).toHaveBeenCalledWith("report_thumbnail_save", { reportId: ID, imageBase64: "iVBORw==" });
    // The library holds the report with its new image version, and the frame is gone.
    expect(getReportsSnapshot().reports[0].thumbnail_updated_at_ms).toBe(7n);
    expect(captureFrame()).toBeNull();
  });

  it("ignores an answer under another nonce and fails when the page never answers", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    answer({ report_thumbnail_capture_open: () => page });
    const capture = captureReportPreview(ID);
    await pageAnswers(() => ({ type: "ida-thumbnail", nonce: "forged", image_base64: "iVBORw==" }));
    await vi.advanceTimersByTimeAsync(20_000);

    await expect(capture).rejects.toThrow("timed out");
    expect(invoke).not.toHaveBeenCalledWith("report_thumbnail_save", expect.anything());
    expect(captureFrame()).toBeNull();
  });

  it("stores nothing when the page reports that its capture failed", async () => {
    answer({ report_thumbnail_capture_open: () => page });
    const capture = captureReportPreview(ID);
    await pageAnswers((nonce) => ({ type: "ida-thumbnail", nonce, error: "Thumbnail capture failed" }));

    await expect(capture).rejects.toThrow("capture failed");
    expect(invoke).not.toHaveBeenCalledWith("report_thumbnail_save", expect.anything());
  });

  it("stops and removes its frame when aborted", async () => {
    answer({ report_thumbnail_capture_open: () => page });
    const abort = new AbortController();
    const capture = captureReportPreview(ID, abort.signal);
    await vi.waitFor(() => expect(captureFrame()).not.toBeNull());
    abort.abort(new Error("left Mine"));

    await expect(capture).rejects.toThrow("left Mine");
    expect(captureFrame()).toBeNull();
  });

  it("runs one capture at a time", async () => {
    answer({
      report_thumbnail_capture_open: (args) => ({ ...page, id: args.reportId }),
      report_thumbnail_save: (args) => ({ ...saved, id: args.reportId }),
    });
    const first = captureReportPreview("r1");
    const second = captureReportPreview("r2");
    await pageAnswers((nonce) => ({ type: "ida-thumbnail", nonce, image_base64: "iVBORw==" }));
    await expect(first).resolves.toMatchObject({ id: "r1" });
    // The second starts only once the first has finished.
    expect(invoke.mock.calls.map(([command]) => command)).toEqual([
      "report_thumbnail_capture_open",
      "report_thumbnail_save",
      "report_thumbnail_capture_open",
    ]);
    await pageAnswers((nonce) => ({ type: "ida-thumbnail", nonce, image_base64: "iVBORw==" }));
    await expect(second).resolves.toMatchObject({ id: "r2" });
  });
});

describe("captureMissingPreviewOnce", () => {
  const report = (patch: object) =>
    ({ id: ID, kind: "dashboard", is_owner: true, thumbnail_updated_at_ms: null, ...patch }) as never;

  it("captures an owner's dashboard with no image, once per session", async () => {
    answer({ report_thumbnail_capture_open: () => page, report_thumbnail_save: () => saved });
    captureMissingPreviewOnce(report({}));
    captureMissingPreviewOnce(report({}));
    await pageAnswers((nonce) => ({ type: "ida-thumbnail", nonce, image_base64: "iVBORw==" }));
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith("report_thumbnail_save", expect.anything()));
    expect(invoke.mock.calls.filter(([command]) => command === "report_thumbnail_capture_open")).toHaveLength(1);
  });

  it("leaves a report it cannot or need not capture alone", async () => {
    captureMissingPreviewOnce(report({ is_owner: false }));
    captureMissingPreviewOnce(report({ kind: "workbook" }));
    captureMissingPreviewOnce(report({ thumbnail_updated_at_ms: 3n }));
    await Promise.resolve();
    expect(invoke).not.toHaveBeenCalled();
  });
});
