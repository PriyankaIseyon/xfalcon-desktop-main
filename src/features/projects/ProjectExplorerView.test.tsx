import { useState } from "react";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ProjectDirectoryEntry, ProjectView } from "../../lib/generated/contracts";
import { ProjectExplorerView } from "./ProjectExplorerView";
import { MAX_AUTO_EXPANDED_DIRECTORIES } from "./useProjectTree";

const projectListDirectoryMock = vi.hoisted(() => vi.fn());
const projectFileStatMock = vi.hoisted(() => vi.fn());
const projectFileReadBytesMock = vi.hoisted(() => vi.fn());
const projectSchemaSummaryMock = vi.hoisted(() => vi.fn());

vi.mock("../../lib/native", () => ({
  projectListDirectory: projectListDirectoryMock,
  PROJECT_READ_RANGE_MAX_BYTES: 8 * 1024 * 1024,
  projectFileStat: projectFileStatMock,
  projectFileReadBytes: projectFileReadBytesMock,
  projectSchemaSummary: projectSchemaSummaryMock,
}));

const project: ProjectView = {
  version: 1,
  id: "project-1",
  display_name: "Research",
  root_path: "/home/user/research",
  status: "trusted",
  active: true,
  trusted_at_ms: 0,
};

function entry(overrides: Partial<ProjectDirectoryEntry> & { name: string }): ProjectDirectoryEntry {
  return { kind: "file", size_bytes: 0, is_sensitive: false, ...overrides };
}

function listing(path: string | null, entries: ProjectDirectoryEntry[]) {
  return { version: 1, project_id: project.id, path, entries };
}

afterEach(() => {
  cleanup();
  projectListDirectoryMock.mockReset();
  projectFileStatMock.mockReset();
  projectFileReadBytesMock.mockReset();
  projectSchemaSummaryMock.mockReset();
});

/**
 * The view is controlled: the shell owns which file is open, because the
 * sidebar opens files too. This holds that state for the tests so a click in
 * the tree still lands in the viewer.
 */
function Harness({ project: forProject }: { project: ProjectView | null }) {
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  return (
    <ProjectExplorerView
      onProjectReady={() => {}}
      onSelectPath={setSelectedPath}
      project={forProject}
      selectedPath={selectedPath}
    />
  );
}

describe("ProjectExplorerView", () => {
  it("says no project is open instead of showing an empty tree", () => {
    render(<Harness project={null} />);

    expect(screen.getByText("No project is open")).toBeInTheDocument();
    expect(projectListDirectoryMock).not.toHaveBeenCalled();
  });

  it("opens folders by default, one listing per folder", async () => {
    projectListDirectoryMock.mockImplementation(async (_projectId: string, path: string | null) => {
      if (path === null) {
        return listing(null, [entry({ name: "src", kind: "directory" }), entry({ name: "README.md" })]);
      }
      if (path === "src") return listing("src", [entry({ name: "main.ts" })]);
      throw new Error(`Unexpected listing for ${String(path)}`);
    });

    render(<Harness project={project} />);

    expect(await screen.findByRole("treeitem", { name: /main\.ts/ })).toBeInTheDocument();
    expect(screen.getByRole("treeitem", { name: /src/ })).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByRole("treeitem", { name: /README\.md/ })).toBeInTheDocument();
    // A child draws branches from its folder, so the folder reads as its parent.
    expect(screen.getByRole("treeitem", { name: /main\.ts/ }).querySelectorAll(".project-tree__guide")).toHaveLength(1);
    expect(screen.getByRole("treeitem", { name: /README\.md/ }).querySelectorAll(".project-tree__guide")).toHaveLength(
      0,
    );
    expect(projectListDirectoryMock).toHaveBeenCalledWith("project-1", null);
    expect(projectListDirectoryMock).toHaveBeenCalledWith("project-1", "src");
    expect(projectListDirectoryMock).toHaveBeenCalledTimes(2);
  });

  it("does not open dot-folders or dependency folders by default", async () => {
    projectListDirectoryMock.mockImplementation(async (_projectId: string, path: string | null) => {
      if (path === null) {
        return listing(null, [
          entry({ name: ".git", kind: "directory" }),
          entry({ name: "node_modules", kind: "directory" }),
          entry({ name: "src", kind: "directory" }),
        ]);
      }
      if (path === "src") return listing("src", [entry({ name: "main.ts" })]);
      throw new Error(`Unexpected listing for ${String(path)}`);
    });

    render(<Harness project={project} />);

    await screen.findByRole("treeitem", { name: /main\.ts/ });
    expect(screen.getByRole("treeitem", { name: /node_modules/ })).toHaveAttribute("aria-expanded", "false");
    expect(screen.getByRole("treeitem", { name: /\.git/ })).toHaveAttribute("aria-expanded", "false");
    expect(projectListDirectoryMock).toHaveBeenCalledTimes(2);
  });

  it("stops opening folders by default once its budget is spent", async () => {
    const folders = Array.from({ length: MAX_AUTO_EXPANDED_DIRECTORIES + 5 }, (_, index) =>
      entry({ name: `d${String(index).padStart(3, "0")}`, kind: "directory" }),
    );
    projectListDirectoryMock.mockImplementation(async (_projectId: string, path: string | null) =>
      path === null ? listing(null, folders) : listing(path, []),
    );

    render(<Harness project={project} />);

    await waitFor(() => expect(projectListDirectoryMock).toHaveBeenCalledTimes(MAX_AUTO_EXPANDED_DIRECTORIES + 1));
    // The rows past the budget stay closed: their folders are never read.
    expect(projectListDirectoryMock).not.toHaveBeenCalledWith("project-1", folders[folders.length - 1].name);
  });

  it("collapses every folder at once, then expands them again from one control", async () => {
    projectListDirectoryMock.mockImplementation(async (_projectId: string, path: string | null) =>
      path === null
        ? listing(null, [entry({ name: "src", kind: "directory" })])
        : listing("src", [entry({ name: "main.ts" })]),
    );
    const user = userEvent.setup();

    render(<Harness project={project} />);
    await screen.findByRole("treeitem", { name: /main\.ts/ });

    await user.click(screen.getByRole("button", { name: "Collapse all folders" }));

    await waitFor(() => expect(screen.queryByRole("treeitem", { name: /main\.ts/ })).not.toBeInTheDocument());
    expect(screen.getByRole("treeitem", { name: /src/ })).toHaveAttribute("aria-expanded", "false");
    // The same control now expands, from the cached listing.
    await user.click(screen.getByRole("button", { name: "Expand all folders" }));
    expect(await screen.findByRole("treeitem", { name: /main\.ts/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Collapse all folders" })).toBeInTheDocument();
    expect(projectListDirectoryMock).toHaveBeenCalledTimes(2);
  });

  it("re-uses a folder's cached children instead of re-reading it on every expand", async () => {
    projectListDirectoryMock.mockImplementation(async (_projectId: string, path: string | null) =>
      path === null
        ? listing(null, [entry({ name: "src", kind: "directory" })])
        : listing("src", [entry({ name: "main.ts" })]),
    );
    const user = userEvent.setup();

    render(<Harness project={project} />);
    await screen.findByRole("treeitem", { name: /main\.ts/ });
    const folder = screen.getByRole("treeitem", { name: /src/ });

    await user.click(folder);
    await waitFor(() => expect(screen.queryByRole("treeitem", { name: /main\.ts/ })).not.toBeInTheDocument());
    await user.click(folder);
    await screen.findByRole("treeitem", { name: /main\.ts/ });

    expect(projectListDirectoryMock).toHaveBeenCalledTimes(2);
  });

  it("names a row by its whole file name when the drawn name is cut in the middle", async () => {
    const long = "Global Geography Trivia Dashboard for the Quarterly Review.html";
    projectListDirectoryMock.mockResolvedValue(listing(null, [entry({ name: long })]));

    render(<Harness project={project} />);

    const row = await screen.findByRole("treeitem", { name: new RegExp(`^${long.replace(/\./g, "\\.")}`) });
    expect(row).toHaveTextContent("…");
  });

  it("does not start a second folder read when it is reopened before the first read settles", async () => {
    let finishFolder: (value: ReturnType<typeof listing>) => void = () => {};
    projectListDirectoryMock.mockImplementation(async (_projectId: string, path: string | null) => {
      if (path === null) return listing(null, [entry({ name: "src", kind: "directory" })]);
      return new Promise((resolve) => {
        finishFolder = resolve;
      });
    });
    const user = userEvent.setup();

    render(<Harness project={project} />);
    const folder = await screen.findByRole("treeitem", { name: /src/ });
    // The folder opened by default, and its read is in flight.
    await waitFor(() => expect(projectListDirectoryMock).toHaveBeenCalledTimes(2));

    await user.click(folder);
    await user.click(folder);

    expect(projectListDirectoryMock).toHaveBeenCalledTimes(2);
    finishFolder(listing("src", [entry({ name: "main.ts" })]));
    expect(await screen.findByRole("treeitem", { name: /main\.ts/ })).toBeInTheDocument();
  });

  it("marks a sensitive entry in the tree and refuses to open it", async () => {
    projectListDirectoryMock.mockResolvedValueOnce(
      listing(null, [entry({ name: ".env", is_sensitive: true }), entry({ name: "app.ts" })]),
    );

    const user = userEvent.setup();
    render(<Harness project={project} />);

    const sensitive = await screen.findByRole("treeitem", { name: /\.env/ });
    expect(sensitive).toHaveAccessibleName(expect.stringContaining("Classified as sensitive"));
    expect(screen.getByRole("treeitem", { name: /app\.ts$/ })).not.toHaveAccessibleName(
      expect.stringContaining("Classified as sensitive"),
    );

    await user.click(sensitive);
    expect(projectFileStatMock).not.toHaveBeenCalled();
    expect(screen.getByText("No file selected")).toBeInTheDocument();
  });

  it("uses a valid stable DOM id for a path that contains spaces", async () => {
    projectListDirectoryMock.mockResolvedValueOnce(listing(null, [entry({ name: "quarterly report.md" })]));

    render(<Harness project={project} />);

    const row = await screen.findByRole("treeitem", { name: /quarterly report\.md/ });
    const tree = screen.getByRole("tree", { name: "Project files" });
    expect(row.id).not.toMatch(/\s/);
    expect(tree).toHaveAttribute("aria-activedescendant", row.id);
  });

  it("shows a symlink and a non-regular file but will not open either", async () => {
    projectListDirectoryMock.mockResolvedValueOnce(
      listing(null, [entry({ name: "link", kind: "symlink" }), entry({ name: "socket", kind: "other" })]),
    );
    const user = userEvent.setup();

    render(<Harness project={project} />);

    const link = await screen.findByRole("treeitem", { name: /link/ });
    expect(link).toHaveAccessibleName(expect.stringContaining("not followed"));
    expect(screen.getByRole("treeitem", { name: /socket/ })).toHaveAccessibleName(
      expect.stringContaining("Not a regular file"),
    );

    await user.click(link);

    expect(projectFileStatMock).not.toHaveBeenCalled();
    expect(screen.getByText("No file selected")).toBeInTheDocument();
  });

  it("opens a file from the keyboard alone", async () => {
    projectListDirectoryMock.mockResolvedValueOnce(listing(null, [entry({ name: "a.txt" }), entry({ name: "b.txt" })]));
    // Every stat answers the same: the load stats before and after its read.
    projectFileStatMock.mockResolvedValue({
      kind: "text",
      version: 1,
      path: "b.txt",
      size_bytes: 11,
      modified_ms: 1,
    });
    projectFileReadBytesMock.mockResolvedValueOnce(new TextEncoder().encode("second file"));
    const user = userEvent.setup();

    render(<Harness project={project} />);
    await screen.findByRole("treeitem", { name: /a\.txt/ });

    const tree = screen.getByRole("tree", { name: "Project files" });
    tree.focus();
    await user.keyboard("{ArrowDown}{Enter}");

    await waitFor(() => expect(projectFileStatMock).toHaveBeenCalledWith("project-1", "b.txt"));
    expect(await screen.findByText("second file")).toBeInTheDocument();
  });

  it("expands and collapses a folder with the arrow keys", async () => {
    projectListDirectoryMock.mockImplementation(async (_projectId: string, path: string | null) =>
      path === null
        ? listing(null, [entry({ name: "src", kind: "directory" })])
        : listing("src", [entry({ name: "main.ts" })]),
    );
    const user = userEvent.setup();

    render(<Harness project={project} />);
    await screen.findByRole("treeitem", { name: /main\.ts/ });

    screen.getByRole("tree", { name: "Project files" }).focus();
    await user.keyboard("{ArrowLeft}");

    await waitFor(() => expect(screen.queryByRole("treeitem", { name: /main\.ts/ })).not.toBeInTheDocument());
    expect(screen.getByRole("treeitem", { name: /src/ })).toHaveAttribute("aria-expanded", "false");

    await user.keyboard("{ArrowRight}");

    expect(await screen.findByRole("treeitem", { name: /main\.ts/ })).toBeInTheDocument();
    expect(screen.getByRole("treeitem", { name: /src/ })).toHaveAttribute("aria-expanded", "true");
  });

  it("reports a root listing failure instead of an empty project", async () => {
    projectListDirectoryMock.mockRejectedValueOnce("project is not trusted for the current account");

    render(<Harness project={project} />);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "This folder could not be listed. Project is not trusted for the current account.",
    );
    expect(screen.queryByText("This project folder is empty.")).not.toBeInTheDocument();
  });

  // `useProjectTree` marks a directory expanded the instant it is opened,
  // before its listing settles -- the one optimistic local-state update in
  // this tree. A failed listing must leave that expansion state exactly
  // where the click left it (still expanded, not silently reverted) with a
  // reachable retry, not some third, half-collapsed state nothing else here
  // produces.
  it("keeps a failed subfolder expanded with its own error, and retries cleanly on reopen", async () => {
    let listDirectoryCalls = 0;
    projectListDirectoryMock.mockImplementation(async (_projectId: string, path: string | null) => {
      if (path === null) return listing(null, [entry({ name: "src", kind: "directory" })]);
      listDirectoryCalls += 1;
      if (listDirectoryCalls === 1) return Promise.reject("disk is offline");
      return listing("src", [entry({ name: "main.ts" })]);
    });
    const user = userEvent.setup();

    render(<Harness project={project} />);
    // The folder opens by default, and that first read fails.
    expect(await screen.findByText(/disk is offline/i)).toBeInTheDocument();
    expect(screen.getByRole("treeitem", { name: /src/ })).toHaveAttribute("aria-expanded", "true");
    expect(screen.queryByRole("treeitem", { name: /main\.ts/ })).not.toBeInTheDocument();

    // Collapsing and reopening is the only retry route a nested folder has --
    // there is no dedicated retry action on the row itself.
    await user.click(screen.getByRole("treeitem", { name: /src/ }));
    await waitFor(() =>
      expect(screen.getByRole("treeitem", { name: /src/ })).toHaveAttribute("aria-expanded", "false"),
    );
    await user.click(screen.getByRole("treeitem", { name: /src/ }));

    expect(await screen.findByRole("treeitem", { name: /main\.ts/ })).toBeInTheDocument();
    expect(listDirectoryCalls).toBe(2);
  });

  it("warns when the trusted folder itself is unavailable", async () => {
    projectListDirectoryMock.mockResolvedValueOnce(listing(null, []));

    render(<Harness project={{ ...project, status: "unavailable" }} />);

    expect(await screen.findByText("This project folder is not available")).toBeInTheDocument();
    expect(projectListDirectoryMock).not.toHaveBeenCalled();
  });

  it("distinguishes a genuinely empty project from one that failed to list", async () => {
    projectListDirectoryMock.mockResolvedValueOnce(listing(null, []));

    render(<Harness project={project} />);

    expect(await screen.findByText("This project folder is empty.")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("resets local controls before a different project can use them", async () => {
    const secondProject: ProjectView = {
      ...project,
      id: "project-2",
      display_name: "Operations",
      root_path: "/home/user/operations",
    };
    projectListDirectoryMock.mockImplementation(async (projectId: string, path: string | null) =>
      listing(path, [entry({ name: projectId === project.id ? "research.md" : "operations.md" })]),
    );
    projectSchemaSummaryMock.mockResolvedValue({ version: 1, project_id: project.id, tables: [] });
    const user = userEvent.setup();
    const view = render(<Harness project={project} />);

    await screen.findByRole("treeitem", { name: /research\.md/ });
    await user.type(screen.getByRole("searchbox", { name: "Filter visible project files" }), "missing");
    expect(screen.getByText("No visible files match this filter.")).toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "Schemas" }));
    await waitFor(() => expect(projectSchemaSummaryMock).toHaveBeenCalledWith(project.id));

    view.rerender(<Harness project={secondProject} />);

    expect(await screen.findByRole("treeitem", { name: /operations\.md/ })).toBeInTheDocument();
    expect(screen.getByRole("searchbox", { name: "Filter visible project files" })).toHaveValue("");
    // The new project opens on its files, not on the last project's schemas.
    expect(screen.getByRole("tab", { name: "File" })).toHaveAttribute("aria-selected", "true");
    expect(projectSchemaSummaryMock).toHaveBeenCalledTimes(1);
  });
});
