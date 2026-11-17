import { Channel } from "@tauri-apps/api/core";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it, vi } from "vitest";
import type { ProjectOfficePreviewStarted } from "./generated/contracts";
import { projectOfficePreview, reportOfficePreview } from "./projectOfficePreview";

const ID = "f8479b1e-4c95-45d1-b4cd-b4af44ec5c65";
const pdf = () => {
  // IPC deserializes into the webview's realm. Node's TextEncoder in jsdom
  // returns a different realm's ArrayBuffer, unlike the production boundary.
  const buffer = new ArrayBuffer(12);
  new Uint8Array(buffer).set([37, 80, 68, 70, 45, 49, 46, 55, 32, 116, 101, 115]);
  return buffer;
};

afterEach(() => clearMocks());

function started(channel: Channel<ProjectOfficePreviewStarted>): void {
  channel.onmessage({ version: 1, render_id: ID });
}

it("uses a typed registration channel and returns binary bytes", async () => {
  const handler = vi.fn((command, args) => {
    expect(command).toBe("project_office_preview");
    expect(args.projectId).toBe("project");
    expect(args.path).toBe("deck.pptx");
    expect(args.onStarted).toBeInstanceOf(Channel);
    started(args.onStarted);
    return pdf();
  });
  mockIPC(handler);
  expect(await projectOfficePreview("project", "deck.pptx")).toEqual({ data: new Uint8Array(pdf()), renderId: ID });
  expect(handler).toHaveBeenCalledTimes(1);
});

it("previews a Reports-library file by its report id, and cancels it through the same command", async () => {
  const controller = new AbortController();
  const commands: string[] = [];
  mockIPC((command, args) => {
    commands.push(command);
    if (command === "project_office_preview_cancel") {
      expect(args).toEqual({ renderId: ID });
      return;
    }
    expect(command).toBe("report_office_preview");
    expect(args).toEqual({ reportId: "report-5", onStarted: expect.any(Channel) });
    started((args as { onStarted: Channel<ProjectOfficePreviewStarted> }).onStarted);
    controller.abort();
    return pdf();
  });
  await expect(reportOfficePreview("report-5", controller.signal)).rejects.toMatchObject({ name: "AbortError" });
  expect(commands).toEqual(["report_office_preview", "project_office_preview_cancel"]);
});

it.each(["before registration", "after registration"])("cancels an abandoned request %s", async (timing) => {
  const controller = new AbortController();
  let channel!: Channel<ProjectOfficePreviewStarted>;
  let finish!: (value: ArrayBuffer) => void;
  const commands: string[] = [];
  mockIPC((command, args) => {
    commands.push(command);
    if (command === "project_office_preview_cancel") {
      expect(args).toEqual({ renderId: ID });
      return;
    }
    channel = (args as { onStarted: Channel<ProjectOfficePreviewStarted> }).onStarted;
    return new Promise<ArrayBuffer>((resolve) => {
      finish = resolve;
    });
  });
  const pending = projectOfficePreview("project", "deck.pptx", controller.signal);
  if (timing === "after registration") started(channel);
  controller.abort();
  if (timing === "before registration") started(channel);
  finish(pdf());
  await expect(pending).rejects.toMatchObject({ name: "AbortError" });
  expect(commands).toEqual(["project_office_preview", "project_office_preview_cancel"]);
});

it("surfaces cancellation delivery failure", async () => {
  const controller = new AbortController();
  mockIPC((command, args) => {
    if (command === "project_office_preview_cancel") throw new Error("Cancellation delivery failed");
    started((args as { onStarted: Channel<ProjectOfficePreviewStarted> }).onStarted);
    controller.abort();
    return pdf();
  });
  await expect(projectOfficePreview("project", "deck.pptx", controller.signal)).rejects.toThrow(
    "Cancellation delivery failed",
  );
});

it("does not register a request for an already abandoned view", async () => {
  const handler = vi.fn();
  mockIPC(handler);
  const controller = new AbortController();
  controller.abort();
  await expect(projectOfficePreview("project", "deck.pptx", controller.signal)).rejects.toMatchObject({
    name: "AbortError",
  });
  expect(handler).not.toHaveBeenCalled();
});

it.each([new ArrayBuffer(0), [37, 80, 68, 70], null])("rejects invalid binary IPC responses %#", async (response) => {
  mockIPC(() => response);
  await expect(projectOfficePreview("project", "deck.pptx")).rejects.toThrow("response was invalid");
});
