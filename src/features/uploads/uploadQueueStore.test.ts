import { beforeEach, describe, expect, it } from "vitest";
import type { UploadProjectionV1, UploadQueueV1 } from "../../lib/generated/contracts";
import { applyUploadQueue, getUploadQueue, resetUploadQueue, resetUploadQueueForTests } from "./uploadQueueStore";

function upload(state: UploadProjectionV1["state"]): UploadProjectionV1 {
  return {
    version: 1,
    id: "upload-1",
    purpose: "persistent_dataset",
    state,
    bytes_sent: state === "ready" ? 10 : 0,
    size: 10,
    display_name: "sales.csv",
    safe_error: null,
  };
}

function queue(revision: number, state: UploadProjectionV1["state"]): UploadQueueV1 {
  return { version: 1, revision, uploads: [upload(state)] };
}

beforeEach(resetUploadQueueForTests);

describe("the shared upload queue", () => {
  it("ignores a command snapshot that arrives after a newer event", () => {
    applyUploadQueue(queue(8, "ready"));
    applyUploadQueue(queue(7, "selected"));

    expect(getUploadQueue()).toEqual(queue(8, "ready"));
  });

  it("ignores a conflicting copy of one revision", () => {
    applyUploadQueue(queue(4, "uploading"));
    applyUploadQueue(queue(4, "selected"));

    expect(getUploadQueue()).toEqual(queue(4, "uploading"));
  });

  it("does not let a delayed old-account event repopulate a reset queue", () => {
    applyUploadQueue(queue(8, "ready"));

    resetUploadQueue();
    applyUploadQueue(queue(7, "uploading"));

    expect(getUploadQueue()).toEqual({ version: 1, revision: 8, uploads: [] });
  });
});
