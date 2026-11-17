import { describe, expect, it } from "vitest";
import { DEFAULT_SLASH_COMMANDS, extractSlashCommand, SLASH_COMMAND_KEYS } from "./composerDefaults";

describe("composerDefaults slash command parsing", () => {
  it("matches exact zero-argument slash commands", () => {
    expect(extractSlashCommand("/clear")).toBe("clear");
    expect(extractSlashCommand("/help")).toBe("help");
    expect(extractSlashCommand("/mcp")).toBe("mcp");
    expect(extractSlashCommand("/project")).toBe("project");
    expect(extractSlashCommand("/model")).toBe("model");
    expect(extractSlashCommand("/skill")).toBe("skill");
  });

  it("handles leading and trailing whitespace around the command", () => {
    expect(extractSlashCommand("  /clear  ")).toBe("clear");
    expect(extractSlashCommand("/model ")).toBe("model");
  });

  it("rejects multi-word conversational messages starting with a slash command so prompts are not eaten", () => {
    expect(extractSlashCommand("/mcp servers keep timing out, why?")).toBeNull();
    expect(extractSlashCommand("/project which files did we touch?")).toBeNull();
    expect(extractSlashCommand("/clear the old rows please")).toBeNull();
    expect(extractSlashCommand("/help me write a query")).toBeNull();
    expect(extractSlashCommand("/model gpt-4o")).toBeNull();
    expect(extractSlashCommand("/skill financial-analysis")).toBeNull();
  });

  it("rejects non-command inputs", () => {
    expect(extractSlashCommand("hello world")).toBeNull();
    expect(extractSlashCommand("")).toBeNull();
    expect(extractSlashCommand("/unknown")).toBeNull();
    expect(extractSlashCommand("Can you /clear this?")).toBeNull();
  });

  it("provides matching palette items for all defined keys", () => {
    const titles = DEFAULT_SLASH_COMMANDS.map((item) => item.title.slice(1));
    for (const key of SLASH_COMMAND_KEYS) {
      expect(titles).toContain(key);
    }
  });
});
