import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { appReset } from "../../lib/native";
import { getToasts, resetToastsForTests } from "../../ui/toastStore";
import { DataResetSection } from "./DataResetSection";

vi.mock("../../lib/native", () => ({
  appReset: vi.fn(),
}));

afterEach(cleanup);

beforeEach(() => {
  vi.clearAllMocks();
  resetToastsForTests();
});

function openReset(user: ReturnType<typeof userEvent.setup>) {
  return user.click(screen.getByRole("button", { name: "Reset xFalcon" }));
}

// The trigger and the dialog's confirm button share a name, so look inside the open dialog.
function confirmButton() {
  return within(screen.getByRole("alertdialog", { name: "Reset xFalcon?" })).getByRole("button", {
    name: "Reset xFalcon",
  });
}

describe("DataResetSection", () => {
  it("renders the section with the single reset row and its button", () => {
    render(<DataResetSection />);

    const group = screen.getByRole("group", { name: "Data and reset" });
    expect(group).toHaveAttribute("id", "settings-reset");
    expect(
      screen.getByText(
        "Erases everything xFalcon has stored on this device and starts again, as if it had just been installed.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reset xFalcon" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Clear local data…" })).not.toBeInTheDocument();
  });

  describe("reset xFalcon", () => {
    it("explains what is erased and what stays before anything else", async () => {
      const user = userEvent.setup();
      render(<DataResetSection />);

      await openReset(user);
      const dialog = screen.getByRole("alertdialog", { name: "Reset xFalcon?" });
      expect(dialog).toHaveTextContent("What will be erased");
      expect(dialog).toHaveTextContent("What stays");
      expect(dialog).toHaveTextContent("Your account and data on the xFalcon server.");
    });

    it("keeps confirm disabled until the exact text RESET is typed", async () => {
      const user = userEvent.setup();
      render(<DataResetSection />);
      await openReset(user);

      const input = screen.getByLabelText("Type RESET to confirm");
      const confirm = confirmButton();
      expect(confirm).toBeDisabled();

      for (const value of ["reset", " RESET", "RESET "]) {
        await user.clear(input);
        await user.type(input, value);
        expect(confirm).toBeDisabled();
      }

      await user.clear(input);
      await user.type(input, "RESET");
      expect(confirm).toBeEnabled();
      expect(appReset).not.toHaveBeenCalled();
    });

    it("submits on Enter only when the text matches", async () => {
      vi.mocked(appReset).mockReturnValue(new Promise(() => {}));
      const user = userEvent.setup();
      render(<DataResetSection />);
      await openReset(user);

      const input = screen.getByLabelText("Type RESET to confirm");
      await user.type(input, "reset{Enter}");
      expect(appReset).not.toHaveBeenCalled();

      await user.clear(input);
      await user.type(input, "RESET{Enter}");
      expect(appReset).toHaveBeenCalledOnce();
    });

    it("calls the reset once even when confirm is clicked twice", async () => {
      vi.mocked(appReset).mockReturnValue(new Promise(() => {}));
      const user = userEvent.setup();
      render(<DataResetSection />);
      await openReset(user);
      await user.type(screen.getByLabelText("Type RESET to confirm"), "RESET");

      const confirm = confirmButton();
      await user.click(confirm);
      await user.click(confirm);

      expect(appReset).toHaveBeenCalledOnce();
    });

    it("shows the rejection inline, keeps the dialog and typed text, and allows a retry", async () => {
      vi.mocked(appReset)
        .mockRejectedValueOnce("Another reset is already running.")
        .mockReturnValueOnce(new Promise(() => {}));
      const user = userEvent.setup();
      render(<DataResetSection />);
      await openReset(user);

      const input = screen.getByLabelText("Type RESET to confirm");
      await user.type(input, "RESET");
      await user.click(confirmButton());

      expect(await screen.findByRole("alert")).toHaveTextContent("Another reset is already running.");
      expect(screen.getByRole("alertdialog", { name: "Reset xFalcon?" })).toBeInTheDocument();
      expect(input).toHaveValue("RESET");
      expect(confirmButton()).toBeEnabled();

      await user.click(confirmButton());
      await waitFor(() => expect(appReset).toHaveBeenCalledTimes(2));
    });

    it("falls back to the default failure message when the rejection is not text", async () => {
      vi.mocked(appReset).mockRejectedValue(new Error("opaque"));
      const user = userEvent.setup();
      render(<DataResetSection />);
      await openReset(user);
      await user.type(screen.getByLabelText("Type RESET to confirm"), "RESET");
      await user.click(confirmButton());

      expect(await screen.findByRole("alert")).toHaveTextContent("xFalcon could not start the reset. Try again.");
      expect(screen.getByRole("alert")).not.toHaveTextContent("Nothing was removed");
    });

    it("stays open and busy after a successful start", async () => {
      vi.mocked(appReset).mockResolvedValue(undefined);
      const user = userEvent.setup();
      render(<DataResetSection />);
      await openReset(user);
      await user.type(screen.getByLabelText("Type RESET to confirm"), "RESET");
      await user.click(confirmButton());

      await waitFor(() => expect(appReset).toHaveBeenCalledOnce());
      const dialog = screen.getByRole("alertdialog", { name: "Reset xFalcon?" });
      expect(dialog).toHaveTextContent("Resetting xFalcon… The app will close and reopen in a moment.");
      expect(confirmButton()).toHaveAttribute("aria-busy", "true");
      expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
      expect(getToasts()).toEqual([]);
    });

    it("ignores Escape while the reset is in progress", async () => {
      vi.mocked(appReset).mockReturnValue(new Promise(() => {}));
      const user = userEvent.setup();
      render(<DataResetSection />);
      await openReset(user);
      await user.type(screen.getByLabelText("Type RESET to confirm"), "RESET");
      await user.click(confirmButton());

      await user.keyboard("{Escape}");
      expect(screen.getByRole("alertdialog", { name: "Reset xFalcon?" })).toBeInTheDocument();
    });

    it("clears the typed text when the dialog is cancelled", async () => {
      const user = userEvent.setup();
      render(<DataResetSection />);
      await openReset(user);
      await user.type(screen.getByLabelText("Type RESET to confirm"), "RESET");
      await user.click(screen.getByRole("button", { name: "Cancel" }));

      await openReset(user);
      expect(screen.getByLabelText("Type RESET to confirm")).toHaveValue("");
    });
  });
});
