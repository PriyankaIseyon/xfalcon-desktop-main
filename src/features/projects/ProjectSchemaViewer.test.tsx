import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import type { ProjectSchemaSummary, ProjectTableSchema } from "../../lib/generated/contracts";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ProjectSchemaViewer } from "./ProjectSchemaViewer";

const mockProjectSchemaSummary = vi.hoisted(() => vi.fn());

function table(name: string, relative_path: string, format = "csv"): ProjectTableSchema {
  return {
    name,
    relative_path,
    format,
    columns: [{ name: `${name}_id`, inferred_type: "integer", sample_values: ["1"] }],
    sample_rows: [],
    row_count_estimate: 1,
  };
}

vi.mock("../../lib/native", () => ({
  projectSchemaSummary: mockProjectSchemaSummary,
}));

describe("ProjectSchemaViewer", () => {
  beforeEach(() => {
    mockProjectSchemaSummary.mockReset();
  });

  afterEach(cleanup);

  it("renders loading state initially", () => {
    mockProjectSchemaSummary.mockReturnValue(new Promise(() => {}));
    render(<ProjectSchemaViewer projectId="proj-123" path={null} />);
    expect(screen.getByText(/Extracting schemas/)).toBeInTheDocument();
  });

  it("shows only the selected file and updates immediately when its full path changes", async () => {
    mockProjectSchemaSummary.mockResolvedValue({
      version: 1,
      project_id: "proj-123",
      tables: [
        table("pit_stops", "f1_mock_data/pit_stops.csv"),
        table("archived_pit_stops", "archive/pit_stops.csv"),
        table("circuits", "f1_mock_data/circuits.csv"),
      ],
    } satisfies ProjectSchemaSummary);
    const view = render(<ProjectSchemaViewer projectId="proj-123" path="f1_mock_data/pit_stops.csv" />);
    expect(await screen.findByRole("region", { name: "pit_stops" })).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "archived_pit_stops" })).not.toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "circuits" })).not.toBeInTheDocument();

    view.rerender(<ProjectSchemaViewer projectId="proj-123" path={"archive\\pit_stops.csv"} />);
    expect(screen.getByRole("region", { name: "archived_pit_stops" })).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "pit_stops" })).not.toBeInTheDocument();
    expect(mockProjectSchemaSummary).toHaveBeenCalledTimes(1);

    view.rerender(<ProjectSchemaViewer projectId="proj-123" path="notes.md" />);
    expect(screen.getByText("No CSV, JSON, or SQLite schema was found for this file.")).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Schemas" })).not.toBeInTheDocument();
  });

  it("keeps every table belonging to the selected SQLite file", async () => {
    mockProjectSchemaSummary.mockResolvedValue({
      version: 1,
      project_id: "proj-123",
      tables: [
        table("drivers", "race.sqlite", "sqlite"),
        table("stops", "race.sqlite", "sqlite"),
        table("other", "other.sqlite", "sqlite"),
      ],
    } satisfies ProjectSchemaSummary);
    render(<ProjectSchemaViewer projectId="proj-123" path="race.sqlite" />);
    expect(await screen.findByRole("region", { name: "drivers" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "stops" })).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "other" })).not.toBeInTheDocument();
  });

  it("uses the latest file selection when an outstanding extraction finishes", async () => {
    let resolve!: (summary: ProjectSchemaSummary) => void;
    mockProjectSchemaSummary.mockReturnValue(
      new Promise<ProjectSchemaSummary>((done) => {
        resolve = done;
      }),
    );
    const view = render(<ProjectSchemaViewer projectId="proj-123" path="first.json" />);
    view.rerender(<ProjectSchemaViewer projectId="proj-123" path="second.json" />);
    await act(async () =>
      resolve({
        version: 1,
        project_id: "proj-123",
        tables: [table("first", "first.json", "json"), table("second", "second.json", "json")],
      }),
    );
    expect(screen.getByRole("region", { name: "second" })).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "first" })).not.toBeInTheDocument();
  });

  it("renders empty message when no schemas found", async () => {
    mockProjectSchemaSummary.mockResolvedValueOnce({
      version: 1,
      project_id: "proj-123",
      tables: [],
    });
    render(<ProjectSchemaViewer projectId="proj-123" path={null} />);

    await waitFor(() => {
      expect(screen.getByText(/No CSV, JSON, or SQLite schemas/)).toBeInTheDocument();
    });
  });

  it("renders table schemas and columns", async () => {
    mockProjectSchemaSummary.mockResolvedValueOnce({
      version: 1,
      project_id: "proj-123",
      tables: [
        {
          name: "users",
          relative_path: "data/users.csv",
          format: "csv",
          columns: [
            { name: "id", inferred_type: "integer", sample_values: ["1", "2"] },
            { name: "username", inferred_type: "string", sample_values: ["alice", "bob"] },
          ],
          sample_rows: [
            ["1", "alice"],
            ["2", "bob"],
          ],
          row_count_estimate: 2,
        },
      ],
    });

    render(<ProjectSchemaViewer projectId="proj-123" path={null} />);

    await waitFor(() => {
      expect(screen.getByRole("region", { name: "Schemas" })).toBeInTheDocument();
      // The tab names the panel; it repeats no "Schemas · 1 table" heading.
      expect(screen.queryByText("1 table")).not.toBeInTheDocument();
      expect(screen.getByText("users")).toBeInTheDocument();
      expect(screen.getByText("CSV")).toBeInTheDocument();
      expect(screen.getByText("id")).toBeInTheDocument();
      expect(screen.getByText("integer")).toBeInTheDocument();
      expect(screen.getByText("username")).toBeInTheDocument();
      expect(screen.getByText("string")).toBeInTheDocument();
    });
  });

  it("names a top-level CSV once and shows each distinct sample once", async () => {
    mockProjectSchemaSummary.mockResolvedValueOnce({
      version: 1,
      project_id: "proj-123",
      tables: [
        {
          name: "trivia",
          relative_path: "trivia.csv",
          format: "csv",
          columns: [{ name: "Difficulty", inferred_type: "string", sample_values: ["Easy", "Easy", "Hard"] }],
          sample_rows: [],
          row_count_estimate: 50,
        },
      ],
    });

    render(<ProjectSchemaViewer projectId="proj-123" path={null} />);

    const card = await screen.findByRole("region", { name: "trivia" });
    expect(card).not.toHaveTextContent("trivia.csv");
    expect(card).toHaveTextContent("about 50 rows");
    const samples = screen.getByRole("list", { name: "Difficulty samples" });
    expect(Array.from(samples.children, (item) => item.textContent)).toEqual(["Easy", "Hard"]);
  });

  it("shows where a table lives when its name does not say it", async () => {
    mockProjectSchemaSummary.mockResolvedValueOnce({
      version: 1,
      project_id: "proj-123",
      tables: [
        {
          name: "orders",
          relative_path: "shop.sqlite",
          format: "sqlite",
          columns: [{ name: "id", inferred_type: "integer", sample_values: [] }],
          sample_rows: [],
          row_count_estimate: null,
        },
        {
          name: "users",
          relative_path: "data/users.csv",
          format: "csv",
          columns: [{ name: "id", inferred_type: "integer", sample_values: ["1"] }],
          sample_rows: [],
          row_count_estimate: null,
        },
      ],
    });

    render(<ProjectSchemaViewer projectId="proj-123" path={null} />);

    expect(await screen.findByRole("region", { name: "orders" })).toHaveTextContent("shop.sqlite");
    expect(screen.getByRole("region", { name: "users" })).toHaveTextContent("data/users.csv");
  });
});
