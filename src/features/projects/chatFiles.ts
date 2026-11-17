import type { ChatToolCallView } from "../../lib/generated/contracts";

/**
 * The durable wire names whose paths name a file the chat itself used — the
 * `WIRE_*` constants in `src-tauri/src/tools/dispatch.rs`.
 *
 * `project_list_directory` is deliberately absent. Its paths are whatever a
 * folder happens to contain, directories included, so counting them as files
 * the chat used would fill the list with names it only glanced at and would
 * hand the file viewer a directory to open.
 *
 * MCP tools are absent for the reason the backend refuses to summarize them at
 * all (`docs/threat-model.md` TM-006): a server's claim about this
 * filesystem is not this app's claim, and a sidebar row is this app speaking.
 */
export const CHAT_FILE_TOOLS: ReadonlySet<string> = new Set([
  "project_read_file",
  "project_search_files",
  "project_search_text",
  "project_propose_patch",
]);

/**
 * Rows one chat contributes to the sidebar. A long chat can touch hundreds of
 * files; a nav tree that grows without bound stops being a nav tree, and the
 * project's own file view is one row away for the rest.
 */
export const CHAT_FILE_ROW_CAP = 12;

export interface ChatFile {
  path: string;
  /** The classifier's verdict, carried so a sensitive file is drawn as one. */
  sensitive: boolean;
}

export interface ChatFileList {
  /** Most recently touched first, deduplicated, at most `CHAT_FILE_ROW_CAP`. */
  files: readonly ChatFile[];
  /** Distinct paths this chat touched beyond the cap. */
  hidden: number;
}

export const NO_CHAT_FILES: ChatFileList = { files: [], hidden: 0 };

/** The one member of a projected message this derivation reads. */
export interface ChatFileSource {
  readonly toolCalls: readonly ChatToolCallView[];
}

/**
 * The files one conversation used, newest touch first.
 *
 * Newest first because a chat's tail is what you are working on; a chat that
 * read forty files an hour ago and wrote one just now should show the one it
 * wrote at the top. Deduplicated by path, so re-reading a file to check an
 * edit does not push everything else down.
 *
 * Paths come from the step summary the backend composed against the outcome
 * (`tool_step_summary`), which already re-validated each one as
 * project-relative and dropped whatever failed. Nothing is re-derived here.
 */
export function chatFiles(messages: readonly ChatFileSource[]): ChatFileList {
  const sensitiveByPath = new Map<string, boolean>();
  for (let message = messages.length - 1; message >= 0; message -= 1) {
    const calls = messages[message].toolCalls;
    for (let call = calls.length - 1; call >= 0; call -= 1) {
      if (!CHAT_FILE_TOOLS.has(calls[call].name)) continue;
      for (const entry of calls[call].step?.paths ?? []) {
        if (!sensitiveByPath.has(entry.path)) sensitiveByPath.set(entry.path, entry.is_sensitive);
      }
    }
  }
  const distinct = [...sensitiveByPath];
  return {
    files: distinct.slice(0, CHAT_FILE_ROW_CAP).map(([path, sensitive]) => ({ path, sensitive })),
    hidden: Math.max(0, distinct.length - CHAT_FILE_ROW_CAP),
  };
}
