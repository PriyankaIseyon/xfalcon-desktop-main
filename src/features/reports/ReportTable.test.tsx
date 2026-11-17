import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { ReportView } from "../../lib/generated/contracts";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({
  invoke,
  convertFileSrc: (id: string) => `http://xf-preview.localhost/${id}`,
}));

import { ReportTable } from "./ReportTable";

const report = (patch: Partial<ReportView> = {}): ReportView => ({
  version: 1,
  id: "r1",
  kind: "dashboard",
  title: "Pipeline",
  description: null,
  created_at_ms: 1_783_000_000_000n,
  updated_at_ms: 1_783_100_000_000n,
  thumbnail_updated_at_ms: null,
  pinned: false,
  hidden: false,
  byte_size: 100n,
  scope: "global",
  is_owner: true,
  owner_display_name: "Jeevan TM",
  refreshable: true,
  can_refresh: true,
  can_manage: true,
  can_delete: true,
  page_count: 2,
  dataset_count: 1,
  data_sources: ["KPI_LAKE_HOUSE", "Finance"],
  source_conversation_id: null,
  ...patch,
});

const actions = () => ({
  onTogglePin: vi.fn(),
  onToggleHidden: vi.fn(),
  onShare: vi.fn(),
  onRename: vi.fn(),
  onDelete: vi.fn(),
  onRegeneratePreview: vi.fn(),
});

beforeEach(() => {
  invoke.mockReset();
});
afterEach(cleanup);

function show(reports: ReportView[], onOpen = vi.fn(), handlers = actions()) {
  render(
    <ReportTable
      actions={handlers}
      busyById={{}}
      projectOrigins={[{ version: 1, report_id: "r1", project_id: "p1", project_name: "Sales" }]}
      reports={reports}
      onOpen={onOpen}
    />,
  );
  return { onOpen, handlers };
}

const rowOf = (title: string) =>
  screen.getByRole("button", { name: new RegExp(`^Open ${title}`) }).closest("tr") as HTMLElement;

it("gives a row everything a card has: where its data comes from, its facts, and every action", () => {
  show([report()]);
  const row = rowOf("Pipeline");

  // The data sources as DA names them, and the project; never the kind or "Live".
  expect(within(row).getByText("KPI_LAKE_HOUSE, Finance")).toBeInTheDocument();
  expect(within(row).getByText("Sales")).toBeInTheDocument();
  for (const word of [/dashboard/i, /\blive\b/i, /snapshot/i]) expect(row).not.toHaveTextContent(word);
  expect(row).toHaveTextContent("Jeevan TM");
  expect(row).toHaveTextContent("2 pages");
  expect(row).toHaveTextContent("Organization");
  // Pin, hide, share, preview, rename, delete: the six a card offers.
  expect(
    within(row)
      .getAllByRole("button")
      .filter((b) => b.textContent !== "Pipeline"),
  ).toHaveLength(6);
});

it("opens on a click anywhere in the row, but not on one of its controls", async () => {
  const user = userEvent.setup();
  const { onOpen, handlers } = show([report()]);
  const row = rowOf("Pipeline");

  await user.click(within(row).getByText("2 pages"));
  expect(onOpen).toHaveBeenCalledTimes(1);
  await user.click(within(row).getByRole("button", { name: /^Pin / }));
  expect(handlers.onTogglePin).toHaveBeenCalledTimes(1);
  expect(onOpen).toHaveBeenCalledTimes(1);
});

it("offers a reader who does not own a report only its pin and hide, and never draws a dashboard to fill a row", () => {
  show([
    report({
      refreshable: false,
      data_sources: [],
      is_owner: false,
      owner_display_name: null,
      scope: "user",
      can_manage: false,
      can_delete: false,
    }),
  ]);
  const row = rowOf("Pipeline");

  expect(row).toHaveTextContent("Unknown owner");
  expect(row).toHaveTextContent("Private");
  const controls = within(row)
    .getAllByRole("button")
    .filter((b) => b.textContent !== "Pipeline");
  expect(controls.map((b) => b.getAttribute("aria-label")?.split(" ")[0])).toEqual(["Pin", "Hide"]);
  expect(within(row).queryByRole("button", { name: /^(Rename|Delete|Share) / })).toBeNull();
  // The actions column is sized to the two controls it holds, not six.
  expect(row.closest("table")?.style.getPropertyValue("--reports-action-slots")).toBe("2");
  expect(row.querySelector("iframe")).toBeNull();
  expect(invoke).not.toHaveBeenCalledWith("report_open", expect.anything());
});

it("sorts by name, owner, created and updated from the headers, and gives the order back on a third press", async () => {
  const user = userEvent.setup();
  show([
    report({ id: "b", title: "beta", owner_display_name: "Zed", created_at_ms: 3n, updated_at_ms: 10n }),
    report({ id: "a", title: "Alpha", owner_display_name: "amy", created_at_ms: 1n, updated_at_ms: 30n }),
    report({ id: "c", title: "Gamma 10", owner_display_name: null, created_at_ms: 2n, updated_at_ms: 20n }),
  ]);
  const order = () =>
    screen
      .getAllByRole("button", { name: /^Open / })
      .map((b) => b.textContent)
      .join(",");
  const header = (name: string) => screen.getByRole("button", { name: new RegExp(`^${name}$`) });
  const sortOf = (name: string) => header(name).closest("th")?.getAttribute("aria-sort");

  expect(order()).toBe("beta,Alpha,Gamma 10");
  expect(sortOf("Name")).toBe("none");

  await user.click(header("Name"));
  expect(order()).toBe("Alpha,beta,Gamma 10");
  expect(sortOf("Name")).toBe("ascending");
  await user.click(header("Name"));
  expect(order()).toBe("Gamma 10,beta,Alpha");
  expect(sortOf("Name")).toBe("descending");
  await user.click(header("Name"));
  expect(order()).toBe("beta,Alpha,Gamma 10");
  expect(sortOf("Name")).toBe("none");

  // An owner the reader owns reads as "You", and sorts as it reads.
  await user.click(header("Owner"));
  expect(order()).toBe("Alpha,Gamma 10,beta");
  expect(sortOf("Owner")).toBe("ascending");
  expect(sortOf("Name")).toBe("none");

  // Dates sort newest first on the first press.
  await user.click(header("Created"));
  expect(order()).toBe("beta,Gamma 10,Alpha");
  await user.click(header("Updated"));
  expect(order()).toBe("Alpha,Gamma 10,beta");
  await user.click(header("Updated"));
  expect(order()).toBe("beta,Gamma 10,Alpha");
});

const many = (count: number) =>
  Array.from({ length: count }, (_, index) =>
    report({
      id: `r${String(index).padStart(3, "0")}`,
      title: `Report ${String(index + 1).padStart(3, "0")}`,
      updated_at_ms: 1_783_100_000_000n - BigInt(index),
    }),
  );
const titles = () =>
  screen
    .getAllByRole("button", { name: /^Open Report / })
    .map((b) => b.textContent)
    .join(",");

const bar = (placement: "above" | "below") =>
  within(screen.getByRole("navigation", { name: `Pagination ${placement} the table` }));

it("shows 20 rows a page by default, and pages through the rest", async () => {
  const user = userEvent.setup();
  show(many(45));

  expect(screen.getAllByRole("button", { name: /^Open Report / })).toHaveLength(20);
  expect(screen.getAllByText("Showing 1–20 of 45")).toHaveLength(2);
  expect(screen.getAllByRole("navigation", { name: /^Pagination / })[0]).toHaveTextContent("Page 1 of 3");

  await user.click(bar("below").getByRole("button", { name: "Next page" }));
  expect(screen.getAllByText("Showing 21–40 of 45")).toHaveLength(2);
  expect(titles().startsWith("Report 021")).toBe(true);
  await user.click(bar("below").getByRole("button", { name: "Next page" }));
  expect(screen.getAllByText("Showing 41–45 of 45")).toHaveLength(2);
  expect(screen.getAllByRole("button", { name: /^Open Report / })).toHaveLength(5);
  expect(bar("below").getByRole("button", { name: "Next page" })).toBeDisabled();
});

it("pages from the bar above the table and from the bar below it, in step", async () => {
  const user = userEvent.setup();
  show(many(45));

  await user.click(bar("above").getByRole("button", { name: "Next page" }));
  expect(screen.getByRole("navigation", { name: "Pagination below the table" })).toHaveTextContent("Page 2 of 3");
  await user.click(bar("below").getByRole("button", { name: "Previous page" }));
  expect(screen.getByRole("navigation", { name: "Pagination above the table" })).toHaveTextContent("Page 1 of 3");

  // One size choice rules both bars.
  await user.click(screen.getAllByRole("button", { name: "Rows per page" })[0]);
  await user.click(screen.getByRole("menuitemradio", { name: "10" }));
  expect(screen.getAllByText("Showing 1–10 of 45")).toHaveLength(2);
  expect(screen.getAllByRole("button", { name: /^Open Report / })).toHaveLength(10);
});

it("changes the rows a page from the dropdown and starts again at page one", async () => {
  const user = userEvent.setup();
  show(many(120));
  await user.click(bar("above").getByRole("button", { name: "Next page" }));

  await user.click(screen.getAllByRole("button", { name: "Rows per page" })[1]);
  await user.click(screen.getByRole("menuitemradio", { name: "50" }));
  expect(screen.getAllByRole("button", { name: /^Open Report / })).toHaveLength(50);
  expect(screen.getAllByText("Showing 1–50 of 120")).toHaveLength(2);
});

it("sorts every row, not just the page, and begins the sorted list at page one", async () => {
  const user = userEvent.setup();
  show(many(45));
  await user.click(bar("above").getByRole("button", { name: "Next page" }));

  // Name, descending: the last titles come first across the whole library.
  await user.click(screen.getByRole("button", { name: /^Name$/ }));
  await user.click(screen.getByRole("button", { name: /^Name$/ }));
  expect(screen.getAllByText("Showing 1–20 of 45")).toHaveLength(2);
  expect(titles().startsWith("Report 045,Report 044")).toBe(true);
});

it("offers no paging controls for a short table, and never a page past the end", () => {
  const view = render(
    <ReportTable actions={actions()} busyById={{}} projectOrigins={[]} reports={many(8)} onOpen={vi.fn()} />,
  );
  expect(screen.queryByText(/^Showing /)).toBeNull();
  expect(screen.queryByRole("button", { name: "Next page" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Rows per page" })).toBeNull();
  view.unmount();

  const shown = render(
    <ReportTable actions={actions()} busyById={{}} projectOrigins={[]} reports={many(45)} onOpen={vi.fn()} />,
  );
  // 25 reports left after a search: the page the reader was on is clamped, not empty.
  shown.rerender(
    <ReportTable actions={actions()} busyById={{}} projectOrigins={[]} reports={many(12)} onOpen={vi.fn()} />,
  );
  expect(screen.getAllByText("Showing 1–12 of 12")).toHaveLength(2);
});
