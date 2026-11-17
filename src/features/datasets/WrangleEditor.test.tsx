import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { DatasetRecipeStep } from "../../lib/generated/contracts";
import { WrangleEditor } from "./WrangleEditor";

const COLUMNS = ["region", "revenue"];

afterEach(cleanup);

describe("the cleanup editor", () => {
  it("says nothing is changed when there are no steps", () => {
    render(<WrangleEditor columns={COLUMNS} steps={[]} onChange={() => {}} invalidStep={null} disabledReason={null} />);
    expect(screen.getByText(/uploads exactly as it is/)).toBeInTheDocument();
  });

  it("lays out every operation in the same three labelled groups as the web editor", () => {
    render(<WrangleEditor columns={COLUMNS} steps={[]} onChange={() => {}} invalidStep={null} disabledReason={null} />);

    expect(screen.getByRole("group", { name: "Row shape" })).toHaveAttribute("data-step-group", "row");
    expect(screen.getByRole("group", { name: "Column shape" })).toHaveAttribute("data-step-group", "column");
    expect(screen.getByRole("group", { name: "Value tweak" })).toHaveAttribute("data-step-group", "value");
    expect(screen.getAllByRole("button")).toHaveLength(12);
    expect(screen.queryByRole("button", { name: /Add step/ })).not.toBeInTheDocument();
  });

  it("adds a flat operation directly, aimed at the first column", async () => {
    const onChange = vi.fn();
    render(<WrangleEditor columns={COLUMNS} steps={[]} onChange={onChange} invalidStep={null} disabledReason={null} />);

    await userEvent.click(screen.getByRole("button", { name: "Drop duplicates: Remove repeated rows" }));

    expect(onChange).toHaveBeenCalledWith([{ kind: "dedup" }]);
  });

  it("drops the value a null test cannot use", async () => {
    const onChange = vi.fn();
    const steps: DatasetRecipeStep[] = [{ kind: "filter", column: "region", op: "equals", value: "EMEA" }];
    render(
      <WrangleEditor columns={COLUMNS} steps={steps} onChange={onChange} invalidStep={null} disabledReason={null} />,
    );

    await userEvent.click(screen.getByRole("button", { name: "is" }));
    await userEvent.click(screen.getByRole("menuitemradio", { name: "is empty" }));

    // A value left over from the previous comparison would be refused by the
    // backend rather than ignored, so the control clears it.
    expect(onChange).toHaveBeenCalledWith([{ kind: "filter", column: "region", op: "is_null", value: null }]);
  });

  it("reorders steps, because two steps on one column do not commute", async () => {
    const onChange = vi.fn();
    const steps: DatasetRecipeStep[] = [
      { kind: "trim", column: "region" },
      { kind: "upper", column: "region" },
    ];
    render(
      <WrangleEditor columns={COLUMNS} steps={steps} onChange={onChange} invalidStep={null} disabledReason={null} />,
    );

    await userEvent.click(screen.getByRole("button", { name: "Move step 2 earlier" }));
    expect(onChange).toHaveBeenCalledWith([steps[1], steps[0]]);
  });

  it("offers each step the columns that exist at that point in the recipe", () => {
    const steps: DatasetRecipeStep[] = [
      { kind: "filter", column: "region", op: "equals", value: "EMEA" },
      { kind: "rename", column: "region", new_name: "territory" },
      { kind: "trim", column: "territory" },
    ];
    render(
      <WrangleEditor columns={COLUMNS} steps={steps} onChange={() => {}} invalidStep={null} disabledReason={null} />,
    );
    const rows = screen.getAllByRole("listitem");

    expect(rows[0]).toHaveTextContent("region");
    expect(rows[2]).toHaveTextContent("territory");
    expect(rows[2]).not.toHaveTextContent("regionrevenue");
  });

  it("lets a split count be cleared and retyped without changing it to one", async () => {
    const onChange = vi.fn();
    render(
      <WrangleEditor
        columns={COLUMNS}
        steps={[{ kind: "split", column: "region", delimiter: ",", parts: 2 }]}
        onChange={onChange}
        invalidStep={null}
        disabledReason={null}
      />,
    );
    const parts = screen.getByRole("textbox", { name: "Parts" });

    await userEvent.clear(parts);
    expect(onChange).not.toHaveBeenCalled();
    await userEvent.type(parts, "3");

    expect(onChange).toHaveBeenLastCalledWith([{ kind: "split", column: "region", delimiter: ",", parts: 3 }]);
  });

  it("removes a step", async () => {
    const onChange = vi.fn();
    const steps: DatasetRecipeStep[] = [{ kind: "trim", column: "region" }, { kind: "dedup" }];
    render(
      <WrangleEditor columns={COLUMNS} steps={steps} onChange={onChange} invalidStep={null} disabledReason={null} />,
    );

    await userEvent.click(screen.getByRole("button", { name: "Remove step 1" }));
    expect(onChange).toHaveBeenCalledWith([{ kind: "dedup" }]);
  });

  it("marks the step the backend refused", () => {
    render(
      <WrangleEditor
        columns={COLUMNS}
        steps={[{ kind: "trim", column: "region" }, { kind: "dedup" }]}
        onChange={() => {}}
        invalidStep={{ index: 1, message: "Step 2 cannot be applied." }}
        disabledReason={null}
      />,
    );
    expect(screen.getByText("Step 2 cannot be applied.")).toBeInTheDocument();
  });

  it("says why cleanup is unavailable rather than offering controls that cannot work", () => {
    render(
      <WrangleEditor
        columns={COLUMNS}
        steps={[]}
        onChange={() => {}}
        invalidStep={null}
        disabledReason="This file is too large to clean up here."
      />,
    );
    expect(screen.getByText("This file is too large to clean up here.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Filter:/ })).not.toBeInTheDocument();
  });

  it("cannot add any operation when there are no columns to aim one at", () => {
    render(<WrangleEditor columns={[]} steps={[]} onChange={() => {}} invalidStep={null} disabledReason={null} />);
    expect(screen.getAllByRole("button")).toHaveLength(12);
    expect(screen.getAllByRole("button").every((button) => button.hasAttribute("disabled"))).toBe(true);
  });

  it("stops at the recipe bound the backend enforces", () => {
    const steps: DatasetRecipeStep[] = Array.from({ length: 32 }, () => ({ kind: "dedup" as const }));
    render(
      <WrangleEditor columns={COLUMNS} steps={steps} onChange={() => {}} invalidStep={null} disabledReason={null} />,
    );

    expect(screen.getByRole("button", { name: /^Filter:/ })).toBeDisabled();
    expect(screen.getByText("An upload can carry at most 32 cleanup steps.")).toBeInTheDocument();
  });
});
