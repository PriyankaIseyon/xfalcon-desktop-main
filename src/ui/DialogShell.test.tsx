import React from "react";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AlertDialog } from "./AlertDialog";
import { DialogShell } from "./DialogShell";

afterEach(cleanup);

function shell(props: Partial<React.ComponentProps<typeof DialogShell>> = {}) {
  return (
    <DialogShell label="Test dialog" open {...props}>
      <button type="button">Inside</button>
    </DialogShell>
  );
}

describe("DialogShell", () => {
  it("renders through a portal, so no overflow ancestor can clip it", () => {
    const { container } = render(<div data-testid="host">{shell()}</div>);
    const dialog = screen.getByRole("dialog", { name: "Test dialog" });
    expect(container.querySelector("[data-testid=host]")).not.toContainElement(dialog);
    expect(document.body).toContainElement(dialog);
  });

  it("dims with the scrim token rather than with foreground ink", () => {
    render(shell());
    // `bg-foreground/30` is near-white in the dark theme, so it brightened the
    // page instead of dimming it. `--xf-scrim` is defined per theme.
    const scrim = document.querySelector(".dialog-scrim");
    expect(scrim).not.toBeNull();
    expect(scrim?.className).not.toContain("bg-foreground");
  });

  it("closes on Escape and on a scrim click", async () => {
    const user = userEvent.setup();
    const onDismiss = vi.fn();
    render(shell({ onDismiss }));

    await user.keyboard("{Escape}");
    expect(onDismiss).toHaveBeenCalledTimes(1);

    await user.click(document.querySelector(".dialog-scrim")!);
    expect(onDismiss).toHaveBeenCalledTimes(2);
  });

  it("keeps the scrim inert when a stray click must not answer the dialog", async () => {
    const user = userEvent.setup();
    const onDismiss = vi.fn();
    render(shell({ onDismiss, dismissOnScrim: false }));

    await user.click(document.querySelector(".dialog-scrim")!);
    expect(onDismiss).not.toHaveBeenCalled();

    // Escape is a deliberate act, so it still answers.
    await user.keyboard("{Escape}");
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it("refuses both routes when the dialog must be answered", async () => {
    const user = userEvent.setup();
    render(shell());
    await user.keyboard("{Escape}");
    await user.click(document.querySelector(".dialog-scrim")!);
    expect(screen.getByRole("dialog", { name: "Test dialog" })).toBeInTheDocument();
  });

  it("takes the app out of reach while it is open", () => {
    const root = document.createElement("div");
    root.id = "root";
    document.body.append(root);

    const view = render(shell(), { container: root });
    // The Tab trap stops a keyboard at the panel edge; a screen reader's rotor
    // ignores Tab order, so the app root has to be inert as well.
    expect(root).toHaveAttribute("inert");

    view.unmount();
    expect(root).not.toHaveAttribute("inert");
    root.remove();
  });

  it("says alertdialog when the dialog reports a fault", () => {
    render(shell({ role: "alertdialog" }));
    expect(screen.getByRole("alertdialog", { name: "Test dialog" })).toBeInTheDocument();
  });

  it("places a palette high on the screen rather than centred", () => {
    render(shell({ placement: "top" }));
    expect(document.querySelector(".dialog-shell--top")).not.toBeNull();
    expect(document.querySelector(".dialog-shell--center")).toBeNull();
  });
});

describe("AlertDialog", () => {
  it("focuses Cancel before a description link and returns focus to the opener", async () => {
    const user = userEvent.setup();
    function Harness() {
      const [open, setOpen] = React.useState(false);
      return (
        <>
          <button type="button" onClick={() => setOpen(true)}>
            Remove item
          </button>
          <AlertDialog
            open={open}
            title="Remove item?"
            description={<a href="https://example.test/help">Read about removal</a>}
            confirmLabel="Remove"
            onConfirm={vi.fn()}
            onCancel={() => setOpen(false)}
          />
        </>
      );
    }
    render(<Harness />);
    const opener = screen.getByRole("button", { name: "Remove item" });
    await user.click(opener);
    const cancel = screen.getByRole("button", { name: "Cancel" });
    await waitFor(() => expect(cancel).toHaveFocus());
    await user.click(cancel);
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(opener).toHaveFocus();
  });

  it("gives two open dialogs their own labelling ids", () => {
    render(
      <>
        <AlertDialog
          confirmLabel="Delete"
          description="This cannot be undone."
          onCancel={vi.fn()}
          onConfirm={vi.fn()}
          open
          title="Delete project"
        />
        <AlertDialog
          confirmLabel="Sign out"
          description="You will need to sign in again."
          onCancel={vi.fn()}
          onConfirm={vi.fn()}
          open
          title="Sign out"
        />
      </>,
    );

    // A shared constant id pointed both dialogs at the first one's heading.
    const [first, second] = screen.getAllByRole("alertdialog");
    expect(first).toHaveAccessibleName("Delete project");
    expect(second).toHaveAccessibleName("Sign out");
    expect(first).toHaveAccessibleDescription("This cannot be undone.");
    expect(second).toHaveAccessibleDescription("You will need to sign in again.");
  });

  it("cancels on Escape, because the non-destructive outcome is the safe one", async () => {
    const user = userEvent.setup();
    const onCancel = vi.fn();
    const onConfirm = vi.fn();
    render(
      <AlertDialog
        confirmLabel="Delete"
        description="This cannot be undone."
        destructive
        onCancel={onCancel}
        onConfirm={onConfirm}
        open
        title="Delete project"
      />,
    );

    await user.keyboard("{Escape}");
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("keeps a busy action open and reports its failure in the same dialog", async () => {
    const user = userEvent.setup();
    const onCancel = vi.fn();
    render(
      <AlertDialog
        busy
        confirmLabel="Remove"
        description="This removes trust."
        error="The trust store is locked."
        onCancel={onCancel}
        onConfirm={vi.fn()}
        open
        title="Remove project?"
      />,
    );

    expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Remove" })).toHaveAttribute("aria-busy", "true");
    expect(screen.getByRole("alert")).toHaveTextContent("The trust store is locked.");
    await user.keyboard("{Escape}");
    expect(onCancel).not.toHaveBeenCalled();
  });

  it("renders children after the description and before the error", () => {
    render(
      <AlertDialog
        confirmLabel="Reset"
        description="This erases everything."
        error="Nothing was removed."
        onCancel={vi.fn()}
        onConfirm={vi.fn()}
        open
        title="Reset?"
      >
        <p>Body content</p>
      </AlertDialog>,
    );

    const body = screen.getByText("Body content");
    const description = screen.getByText("This erases everything.");
    const error = screen.getByRole("alert");
    expect(description.compareDocumentPosition(body) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(body.compareDocumentPosition(error) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("disables confirm without making the dialog busy, so Cancel stays available", async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    render(
      <AlertDialog
        confirmDisabled
        confirmLabel="Reset"
        description="This erases everything."
        onCancel={onCancel}
        onConfirm={onConfirm}
        open
        title="Reset?"
      />,
    );

    const confirm = screen.getByRole("button", { name: "Reset" });
    expect(confirm).toBeDisabled();
    expect(confirm).not.toHaveAttribute("aria-busy");
    expect(screen.getByRole("button", { name: "Cancel" })).toBeEnabled();
    await user.click(confirm);
    expect(onConfirm).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onCancel).toHaveBeenCalledOnce();
  });
});
