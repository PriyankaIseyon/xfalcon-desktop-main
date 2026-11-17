import { describe, expect, it } from "vitest";
import type { ChatToolCallView, ToolStepPathV1 } from "../../lib/generated/contracts";
import { CHAT_FILE_ROW_CAP, chatFiles } from "./chatFiles";

function call(name: string, paths: string[], sensitive: string[] = []): ChatToolCallView {
  return {
    call_id: `call-${name}-${paths.join(",")}`,
    name,
    status: "answered",
    arguments: null,
    arguments_truncated: false,
    result: null,
    result_truncated: false,
    step: {
      version: 1,
      label: "Read",
      paths: paths.map<ToolStepPathV1>((path) => ({ path, is_sensitive: sensitive.includes(path) })),
      path_total: paths.length,
    },
  };
}

function message(...calls: ChatToolCallView[]) {
  return { toolCalls: calls };
}

describe("chatFiles", () => {
  it("has nothing to show for a chat that ran no tools", () => {
    expect(chatFiles([message()])).toEqual({ files: [], hidden: 0 });
  });

  // Newest first: a chat's tail is what you are working on.
  it("puts the most recently touched file on top", () => {
    const result = chatFiles([
      message(call("project_read_file", ["old.ts"])),
      message(call("project_propose_patch", ["new.ts"])),
    ]);

    expect(result.files.map((file) => file.path)).toEqual(["new.ts", "old.ts"]);
  });

  it("names a file once however often it was touched", () => {
    const result = chatFiles([
      message(call("project_read_file", ["a.ts"])),
      message(call("project_read_file", ["b.ts"])),
      message(call("project_read_file", ["a.ts"])),
    ]);

    expect(result.files.map((file) => file.path)).toEqual(["a.ts", "b.ts"]);
  });

  // A directory listing tells you what exists, not what the chat used -- and
  // its entries include directories, which the file viewer cannot open.
  it("ignores a directory listing's entries", () => {
    const result = chatFiles([message(call("project_list_directory", ["src", "README.md"]))]);

    expect(result.files).toEqual([]);
  });

  // TM-006: a server's claim about this filesystem is not this app's claim.
  it("ignores paths an MCP tool named", () => {
    const result = chatFiles([message(call("mcp_some_server_read", ["/etc/passwd"]))]);

    expect(result.files).toEqual([]);
  });

  it("counts search hits and proposed patches as files the chat used", () => {
    const result = chatFiles([
      message(call("project_search_text", ["hit.ts"]), call("project_search_files", ["found.ts"])),
      message(call("project_propose_patch", ["written.ts"])),
    ]);

    // Within one turn the later call is the newer one, so its paths come first.
    expect(result.files.map((file) => file.path)).toEqual(["written.ts", "found.ts", "hit.ts"]);
  });

  it("carries the classifier's verdict so a sensitive file can be drawn as one", () => {
    const result = chatFiles([message(call("project_search_files", [".env"], [".env"]))]);

    expect(result.files).toEqual([{ path: ".env", sensitive: true }]);
  });

  it("caps the list and says how many it left out", () => {
    const paths = Array.from({ length: CHAT_FILE_ROW_CAP + 3 }, (_, index) => `file-${index}.ts`);
    const result = chatFiles([message(call("project_read_file", paths))]);

    expect(result.files).toHaveLength(CHAT_FILE_ROW_CAP);
    expect(result.hidden).toBe(3);
  });

  it("has nothing to show for a call whose step never landed", () => {
    const unsummarized: ChatToolCallView = { ...call("project_read_file", ["a.ts"]), step: null };

    expect(chatFiles([message(unsummarized)])).toEqual({ files: [], hidden: 0 });
  });
});
