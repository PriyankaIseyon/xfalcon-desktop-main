import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DateField, DateTimeField, TimeField } from "./DateTimeField";
import { Field, Input, Textarea } from "./Field";
import { FileField } from "./FileField";
import { SearchField } from "./SearchField";
import { Select } from "./Select";

afterEach(cleanup);

// ---------------------------------------------------------------------------
// P04-P08: every field wears one surface and says the same things about
// itself. The recurring failure this file exists to stop is a control that
// LOOKS like the others and answers differently -- a label that points at
// nothing, a hint that disappears, a disabled state with a different cursor.
// ---------------------------------------------------------------------------

describe("Field", () => {
  it("points the label at the control without being told an id", () => {
    // A label is only a label when it points at something. Sixteen of twenty
    // call sites hand-wired an id to work around this; the two that did not
    // shipped a caption floating beside an unnamed control.
    render(
      <Field label="Header row">
        <Input />
      </Field>,
    );
    expect(screen.getByLabelText("Header row")).toBe(screen.getByRole("textbox"));
  });

  it("lets an explicit id win over the minted one", () => {
    render(
      <Field htmlFor="worksheet-name" label="Worksheet">
        <Input />
      </Field>,
    );
    expect(screen.getByRole("textbox")).toHaveAttribute("id", "worksheet-name");
    expect(screen.getByLabelText("Worksheet")).toBe(screen.getByRole("textbox"));
  });

  it("keeps an id the control already carries", () => {
    render(
      <Field label="API key">
        <Input id="provider-key" />
      </Field>,
    );
    expect(screen.getByRole("textbox")).toHaveAttribute("id", "provider-key");
  });

  it("keeps the hint when the field also has an error", async () => {
    // The hint used to be replaced by the error, so the sentence explaining
    // the expected format vanished at the moment a person got it wrong.
    render(
      <Field error="That address is not valid" hint="Use your work address" label="Email">
        <Input />
      </Field>,
    );
    const input = screen.getByRole("textbox");
    expect(screen.getByText("Use your work address")).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("That address is not valid");
    const described = (input.getAttribute("aria-describedby") ?? "").split(" ");
    expect(described).toHaveLength(2);
    for (const id of described) expect(document.getElementById(id)).toBeInTheDocument();
    expect(input).toHaveAttribute("aria-invalid", "true");
  });

  it("keeps a description the control brought with it", () => {
    render(
      <>
        <span id="external-note">Stored on this device only</span>
        <Field hint="Use your work address" label="Email">
          <Input aria-describedby="external-note" />
        </Field>
      </>,
    );
    const described = screen.getByRole("textbox").getAttribute("aria-describedby") ?? "";
    expect(described.split(" ")).toContain("external-note");
    expect(described.split(" ")).toHaveLength(2);
  });

  it("marks the control required, not only its label", () => {
    // The asterisk used to be the whole of it: decorative on the label, with
    // nothing on the control for validation or assistive tech to read.
    render(
      <Field label="Name" required>
        <Input />
      </Field>,
    );
    expect(screen.getByRole("textbox")).toBeRequired();
    // ", required", not " (required)": the accessible-name algorithm trims
    // each node, so a leading space is dropped and the note runs into the
    // label. `structureDisplay.test.tsx` owns that rule.
    expect(screen.getByText(", required")).toBeInTheDocument();
  });

  it("says nothing about validity when there is no error", () => {
    render(
      <Field label="Name">
        <Input />
      </Field>,
    );
    const input = screen.getByRole("textbox");
    expect(input).not.toHaveAttribute("aria-invalid");
    expect(input).not.toHaveAttribute("aria-describedby");
    expect(input).not.toBeRequired();
  });

  it("preserves an invalid state supplied by the control", () => {
    render(
      <Field label="Dataset name">
        <Input aria-invalid />
      </Field>,
    );

    expect(screen.getByRole("textbox", { name: "Dataset name" })).toHaveAttribute("aria-invalid", "true");
  });
});

describe("every field wears the one control surface", () => {
  const surface = ["bg-card", "border-default", "rounded-md", "duration-fast", "disabled:cursor-not-allowed"];

  it.each([
    ["text input", <Input key="i" aria-label="Text" />],
    ["textarea", <Textarea key="t" aria-label="Prose" />],
    ["search", <SearchField key="s" aria-label="Search" />],
    ["date", <DateField key="d" aria-label="Date" />],
    ["time", <TimeField key="tm" aria-label="Time" />],
    ["datetime", <DateTimeField key="dt" aria-label="When" />],
  ])("gives the %s the shared surface, transition, and disabled cursor", (_name, element) => {
    const { container } = render(element);
    const control = container.querySelector("input, textarea");
    expect(control).not.toBeNull();
    for (const utility of surface) expect(control!.className).toContain(utility);
  });

  it("gives the select trigger the same surface rather than a copy of it", () => {
    // The copy had no token duration or curve, no invalid treatment, and
    // `pointer-events-none` where every other control says `cursor-not-allowed`.
    render(<Select onChange={() => {}} options={[{ label: "One", value: "1" }]} value={undefined} />);
    const trigger = screen.getByRole("button");
    for (const utility of surface) expect(trigger.className).toContain(utility);
    expect(trigger.className).not.toContain("disabled:pointer-events-none");
  });
});

describe("Select as a field control", () => {
  it("takes the id its label points at, and the state the field injects", () => {
    // A select in a field used to be a control its own label could not name:
    // the label carried `for` and nothing in the document had that id.
    render(
      <Field error="Pick a worksheet" hint="The one uploaded is the one shown" label="Worksheet" required>
        <Select onChange={() => {}} options={[{ label: "Sheet 1", value: "0" }]} value={undefined} />
      </Field>,
    );
    // Announced by the FIELD, not by whichever option happens to be current:
    // `for` names a labelable element and a button is not one, so a select in
    // a field used to announce "Select…, button" and never say which field.
    const trigger = screen.getByRole("button", { name: /^Worksheet/ });
    expect(trigger).toHaveAttribute("aria-required", "true");
    expect(trigger).toHaveAttribute("aria-invalid", "true");
    const described = (trigger.getAttribute("aria-describedby") ?? "").split(" ");
    expect(described).toHaveLength(2);
    for (const id of described) expect(document.getElementById(id)).toBeInTheDocument();
  });

  it("fills the width of the field it sits in", () => {
    const { container } = render(
      <Field label="Worksheet">
        <Select onChange={() => {}} options={[{ label: "Sheet 1", value: "0" }]} value={undefined} />
      </Field>,
    );
    // The menu root shrink-wraps by default, so a full-width trigger inside it
    // was still only as wide as its longest option.
    const root = container.querySelector("span.relative");
    expect(root!.className).toContain("w-full");
  });
});

describe("SearchField", () => {
  it("suppresses the engine's own clear control so there is one", () => {
    // WebView2 is Chromium and draws a clear button inside `type="search"`.
    render(<SearchField aria-label="Search reports" onClear={() => {}} value="sales" />);
    expect(screen.getByRole("searchbox").className).toContain("search-cancel");
  });

  it("offers Clear only when there is something to clear, and names it", async () => {
    const onClear = vi.fn();
    const { rerender } = render(<SearchField aria-label="Search reports" onClear={onClear} value="" />);
    expect(screen.queryByRole("button", { name: "Clear search" })).not.toBeInTheDocument();

    rerender(<SearchField aria-label="Search reports" onClear={onClear} value="sales" />);
    await userEvent.click(screen.getByRole("button", { name: "Clear search" }));
    expect(onClear).toHaveBeenCalledOnce();
  });
});

describe("FileField", () => {
  it("shows keyboard focus on the visible control, not on the hidden input", async () => {
    // The input is `sr-only` and focusable, so the ring lands on a 1px box.
    // The visible label carries it through the peer combinator, which is why
    // the input has to precede the label in the DOM.
    const { container } = render(<FileField label="Choose file" />);
    const input = container.querySelector('input[type="file"]');
    const browse = screen.getByText("Choose file");
    expect(input!.className).toContain("peer");
    expect(browse.className).toContain("peer-focus-visible:outline");
    expect(input!.compareDocumentPosition(browse) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("names its error and points the input at it", () => {
    const { container } = render(<FileField error="That file is too large" label="Choose file" />);
    const input = container.querySelector('input[type="file"]')!;
    expect(input).toHaveAttribute("aria-invalid", "true");
    expect(document.getElementById(input.getAttribute("aria-describedby")!)).toHaveTextContent(
      "That file is too large",
    );
  });
});
