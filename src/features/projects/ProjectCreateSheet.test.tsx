import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ProjectView } from "../../lib/generated/contracts";
import { ProjectCreateSheet } from "./ProjectCreateSheet";

const projectPickFolderMock = vi.hoisted(() => vi.fn());
const projectCreateMock = vi.hoisted(() => vi.fn());
const projectFindByFolderMock = vi.hoisted(() => vi.fn());
const projectActivateMock = vi.hoisted(() => vi.fn());

vi.mock("../../lib/native", () => ({
  projectPickFolder: projectPickFolderMock,
  projectCreate: projectCreateMock,
  projectFindByFolder: projectFindByFolderMock,
  projectActivate: projectActivateMock,
}));

const EXPIRED = "That choice expired. Choose it again.";

/** What the backend's picker hands over: an id, and text to show. */
function folder(displayPath: string, grantId = "grant-1") {
  const name = displayPath.split(/[/\\]/).pop() ?? displayPath;
  return { version: 1, grant_id: grantId, display_name: name, display_path: displayPath };
}

const project: ProjectView = {
  version: 1,
  id: "project-1",
  display_name: "Research",
  root_path: "/home/user/research",
  status: "trusted",
  active: true,
  trusted_at_ms: 0,
};

beforeEach(() => {
  projectFindByFolderMock.mockReset().mockResolvedValue({ version: 1, projects: [] });
  projectActivateMock.mockReset();
});

afterEach(() => {
  cleanup();
  projectPickFolderMock.mockReset();
  projectCreateMock.mockReset();
});

describe("ProjectCreateSheet", () => {
  it("names every matching project and still allows creating a shared-folder project", async () => {
    const user = userEvent.setup();
    const other = { ...project, id: "project-2", display_name: "Forecast" };
    projectFindByFolderMock.mockResolvedValue({ version: 1, projects: [project, other] });
    projectPickFolderMock.mockResolvedValue(folder(project.root_path));
    projectCreateMock.mockResolvedValue({ ...project, id: "project-new" });
    const onReady = vi.fn();
    render(<ProjectCreateSheet open onClose={() => {}} onReady={onReady} />);
    await user.click(screen.getByRole("button", { name: "Choose folder" }));
    expect(projectFindByFolderMock).toHaveBeenCalledWith("grant-1");
    expect(screen.getByText("Shared project folder")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open Research" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open Forecast" })).toBeInTheDocument();
    expect(screen.getByText("File edits in any project affect all projects using this folder.")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Trust & create project" }));
    expect(projectCreateMock).toHaveBeenCalledWith("grant-1", "research");
    expect(projectActivateMock).not.toHaveBeenCalled();
    expect(onReady).toHaveBeenCalledWith(expect.objectContaining({ id: "project-new" }));
  });

  it("opens an existing project without creating one, and keeps activation failures retryable", async () => {
    const user = userEvent.setup();
    projectPickFolderMock.mockResolvedValue(folder(project.root_path));
    projectFindByFolderMock.mockResolvedValue({ version: 1, projects: [project] });
    projectActivateMock.mockRejectedValueOnce(new Error("offline")).mockResolvedValueOnce(project);
    const onReady = vi.fn();
    const onClose = vi.fn();
    render(<ProjectCreateSheet open onClose={onClose} onReady={onReady} />);
    await user.click(screen.getByRole("button", { name: "Choose folder" }));
    await user.click(screen.getByRole("button", { name: "Open Research" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("The project could not be opened");
    expect(onClose).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Open Research" }));
    expect(projectActivateMock).toHaveBeenCalledWith(project.id);
    expect(projectCreateMock).not.toHaveBeenCalled();
    expect(onReady).toHaveBeenCalledWith(project);
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("does not offer creation after a failed folder check and clears old matches for a new folder", async () => {
    const user = userEvent.setup();
    projectPickFolderMock.mockResolvedValue(folder(project.root_path));
    projectFindByFolderMock
      .mockRejectedValueOnce(new Error("unavailable"))
      .mockResolvedValueOnce({ version: 1, projects: [project] })
      .mockResolvedValueOnce({ version: 1, projects: [] });
    render(<ProjectCreateSheet open onClose={() => {}} onReady={() => {}} />);
    await user.click(screen.getByRole("button", { name: "Choose folder" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("The folder could not be checked");
    expect(screen.queryByRole("button", { name: "Trust & create project" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Choose folder" }));
    expect(screen.getByText("Shared project folder")).toBeInTheDocument();
    projectPickFolderMock.mockResolvedValue(folder("/home/user/other", "grant-2"));
    await user.click(screen.getByRole("button", { name: "Choose a different folder" }));
    expect(screen.queryByText("Shared project folder")).not.toBeInTheDocument();
    expect(screen.getByLabelText("Project name")).toHaveValue("other");
    expect(projectCreateMock).not.toHaveBeenCalled();
  });

  it("ignores a folder check that resolves after the sheet reopens", async () => {
    let finish!: (value: { version: 1; projects: ProjectView[] }) => void;
    projectFindByFolderMock.mockReturnValueOnce(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    projectPickFolderMock.mockResolvedValue(folder(project.root_path));
    const props = { onClose: () => {}, onReady: () => {} };
    const { rerender } = render(<ProjectCreateSheet open {...props} />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Choose folder" }));
    expect(screen.queryByRole("button", { name: "Trust & create project" })).not.toBeInTheDocument();
    rerender(<ProjectCreateSheet open={false} {...props} />);
    rerender(<ProjectCreateSheet open {...props} />);
    await act(async () => finish({ version: 1, projects: [project] }));
    expect(screen.queryByText("Shared project folder")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Choose folder" })).toBeEnabled();
  });

  it("shows the trust explanation before create, then confirms with the picked path", async () => {
    const user = userEvent.setup();
    projectPickFolderMock.mockResolvedValueOnce(folder("/home/user/research"));
    projectCreateMock.mockResolvedValueOnce(project);
    const onReady = vi.fn();
    const onClose = vi.fn();

    render(<ProjectCreateSheet open onClose={onClose} onReady={onReady} />);

    expect(screen.queryByText(/Trusting this folder/)).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Choose folder" }));

    expect(await screen.findByText(/Trusting this folder/)).toBeInTheDocument();
    expect(screen.getByText("/home/user/research")).toBeInTheDocument();
    // Explains what trust grants: scoped access, and assistant writes without a review step.
    expect(screen.getByText(/scoped access/)).toBeInTheDocument();
    expect(screen.getByText(/deletes files here automatically.*without asking first/)).toBeInTheDocument();
    expect(screen.getByText(/earlier version is kept under History & Revisions/)).toBeInTheDocument();
    expect(screen.getByText(/file editor are written only when you select Save/)).toBeInTheDocument();
    expect(
      screen.getByText(/permanent deletion of this folder.*removes the project and its chats/),
    ).toBeInTheDocument();

    const nameField = screen.getByLabelText("Project name");
    expect(nameField).toHaveValue("research");

    await user.click(screen.getByRole("button", { name: "Trust & create project" }));

    await waitFor(() => expect(projectCreateMock).toHaveBeenCalledWith("grant-1", "research"));
    expect(onReady).toHaveBeenCalledWith(project);
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("does nothing when the directory picker is cancelled", async () => {
    const user = userEvent.setup();
    projectPickFolderMock.mockResolvedValueOnce(null);
    const onReady = vi.fn();

    render(<ProjectCreateSheet open onClose={() => undefined} onReady={onReady} />);

    await user.click(screen.getByRole("button", { name: "Choose folder" }));

    expect(projectCreateMock).not.toHaveBeenCalled();
    expect(onReady).not.toHaveBeenCalled();
    expect(screen.queryByText(/Trusting this folder/)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Choose folder" })).toBeInTheDocument();
  });

  it("surfaces a create failure inline and keeps the sheet open", async () => {
    const user = userEvent.setup();
    projectPickFolderMock.mockResolvedValueOnce(folder("/home/user/research"));
    projectCreateMock.mockRejectedValueOnce(new Error("the resolved path does not exist"));
    const onReady = vi.fn();
    const onClose = vi.fn();

    render(<ProjectCreateSheet open onClose={onClose} onReady={onReady} />);

    await user.click(screen.getByRole("button", { name: "Choose folder" }));
    await user.click(screen.getByRole("button", { name: "Trust & create project" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("The resolved path does not exist");
    expect(onReady).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
    // The sheet stays open on the review step -- the user can retry without
    // re-picking, and exactly the same path and name are still there to
    // retry with, not reset to the choose-a-folder step.
    expect(screen.getByRole("button", { name: "Trust & create project" })).toBeInTheDocument();
    expect(screen.getByLabelText("Project name")).toHaveValue("research");
    expect(screen.getByText("/home/user/research")).toBeInTheDocument();
  });

  it("renders nothing when closed", () => {
    render(<ProjectCreateSheet open={false} onClose={() => undefined} onReady={() => undefined} />);

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("reports a picker failure on the step where it happened and allows a retry", async () => {
    const user = userEvent.setup();
    projectPickFolderMock
      .mockRejectedValueOnce(new Error("native dialog details"))
      .mockResolvedValueOnce(folder("/home/user/research"));

    render(<ProjectCreateSheet open onClose={() => undefined} onReady={() => undefined} />);

    await user.click(screen.getByRole("button", { name: "Choose folder" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("The folder picker did not open");
    expect(alert).toHaveTextContent("xFalcon could not open the system folder picker.");
    expect(alert).toHaveTextContent("Try again.");
    expect(screen.queryByText(/native dialog details/)).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Choose folder" }));
    expect(await screen.findByText("/home/user/research")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("does not let an old picker result change a newly opened sheet", async () => {
    let resolvePick: (picked: ReturnType<typeof folder>) => void = () => undefined;
    projectPickFolderMock.mockReturnValueOnce(
      new Promise<ReturnType<typeof folder>>((resolve) => {
        resolvePick = resolve;
      }),
    );
    const { rerender } = render(<ProjectCreateSheet open onClose={() => undefined} onReady={() => undefined} />);
    const user = userEvent.setup();

    await user.click(screen.getByRole("button", { name: "Choose folder" }));
    rerender(<ProjectCreateSheet open={false} onClose={() => undefined} onReady={() => undefined} />);
    rerender(<ProjectCreateSheet open onClose={() => undefined} onReady={() => undefined} />);
    await act(async () => resolvePick(folder("/home/user/old-project")));

    expect(screen.queryByText("/home/user/old-project")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Choose folder" })).toBeInTheDocument();
  });

  it("keeps the create operation single and non-dismissible until it resolves", async () => {
    let resolveCreate: (project: ProjectView) => void = () => undefined;
    projectCreateMock.mockReturnValueOnce(
      new Promise<ProjectView>((resolve) => {
        resolveCreate = resolve;
      }),
    );
    projectPickFolderMock.mockResolvedValueOnce(folder("/home/user/research"));
    const onClose = vi.fn();
    const onReady = vi.fn();
    const user = userEvent.setup();
    render(<ProjectCreateSheet open onClose={onClose} onReady={onReady} />);
    await user.click(screen.getByRole("button", { name: "Choose folder" }));

    await user.click(screen.getByRole("button", { name: "Trust & create project" }));
    expect(screen.getByRole("button", { name: "Trust & create project" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Close" })).toBeDisabled();
    await user.keyboard("{Escape}");
    expect(onClose).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Trust & create project" }));
    expect(projectCreateMock).toHaveBeenCalledTimes(1);

    await act(async () => resolveCreate(project));
    expect(onReady).toHaveBeenCalledWith(project);
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("asks the person to choose the folder again when its choice expired before the check", async () => {
    const user = userEvent.setup();
    projectPickFolderMock.mockResolvedValueOnce(folder(project.root_path));
    projectFindByFolderMock.mockRejectedValueOnce(EXPIRED);

    render(<ProjectCreateSheet open onClose={() => {}} onReady={() => {}} />);
    await user.click(screen.getByRole("button", { name: "Choose folder" }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("The folder could not be checked");
    expect(alert).toHaveTextContent(EXPIRED);
    expect(alert).toHaveTextContent("Choose the folder again.");
    expect(screen.queryByRole("button", { name: "Trust & create project" })).not.toBeInTheDocument();
    expect(projectCreateMock).not.toHaveBeenCalled();
  });

  it("refuses to create from an expired choice, names it, and keeps the sheet open", async () => {
    const user = userEvent.setup();
    projectPickFolderMock.mockResolvedValueOnce(folder("/home/user/research"));
    projectCreateMock.mockRejectedValueOnce(EXPIRED);
    const onReady = vi.fn();
    const onClose = vi.fn();

    render(<ProjectCreateSheet open onClose={onClose} onReady={onReady} />);
    await user.click(screen.getByRole("button", { name: "Choose folder" }));
    await user.click(screen.getByRole("button", { name: "Trust & create project" }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("The project was not added");
    expect(alert).toHaveTextContent(EXPIRED);
    expect(alert).toHaveTextContent("Choose the folder again.");
    expect(onReady).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
    expect(projectCreateMock).toHaveBeenCalledWith("grant-1", "research");
    // The expired choice is dropped, so the only way forward is the picker.
    expect(screen.queryByRole("button", { name: "Trust & create project" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Choose folder" })).toBeInTheDocument();
  });
});
