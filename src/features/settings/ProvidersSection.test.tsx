import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ProviderCredentialView } from "../../lib/generated/contracts";
import { Toaster } from "../../ui/Toaster";
import { resetToastsForTests } from "../../ui/toastStore";
import { ProvidersSection } from "./ProvidersSection";

const credentialListMock = vi.hoisted(() => vi.fn());
const credentialCreateMock = vi.hoisted(() => vi.fn());
const credentialReplaceMock = vi.hoisted(() => vi.fn());
const credentialDeleteMock = vi.hoisted(() => vi.fn());

vi.mock("../../lib/native", () => ({
  credentialList: credentialListMock,
  credentialCreate: credentialCreateMock,
  credentialReplace: credentialReplaceMock,
  credentialDelete: credentialDeleteMock,
}));

const openAiKey: ProviderCredentialView = {
  version: 1,
  id: "cred-1",
  provider: "openai",
  display_name: "Work OpenAI",
  status: "ready",
  is_default: true,
};

afterEach(() => {
  cleanup();
  resetToastsForTests();
  credentialListMock.mockReset();
  credentialCreateMock.mockReset();
  credentialReplaceMock.mockReset();
  credentialDeleteMock.mockReset();
});

describe("ProvidersSection", () => {
  it("says plainly that chat cannot run yet and offers the action that fixes it", async () => {
    credentialListMock.mockResolvedValueOnce([]);
    const user = userEvent.setup();

    render(<ProvidersSection />);

    expect(await screen.findByText("No provider keys")).toBeInTheDocument();
    expect(screen.getByText("Add one to enable chat.")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Add provider key" }));

    expect(screen.getByRole("dialog", { name: "Add provider key" })).toBeInTheDocument();
  });

  it("offers the Add button in the header whether or not keys exist, and not while loading", async () => {
    credentialListMock.mockReturnValueOnce(new Promise(() => {}));
    const { unmount } = render(<ProvidersSection />);

    expect(screen.getByText("Loading provider keys…")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Add provider key" })).not.toBeInTheDocument();
    unmount();

    credentialListMock.mockResolvedValueOnce([]);
    const empty = render(<ProvidersSection />);
    expect(await screen.findByText("No provider keys")).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "Add provider key" })).toHaveLength(1);
    empty.unmount();

    credentialListMock.mockResolvedValueOnce([openAiKey]);
    render(<ProvidersSection />);
    expect(await screen.findByText("Work OpenAI")).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "Add provider key" })).toHaveLength(1);
  });

  it("renders the empty state as one compact line with no second Add button", async () => {
    credentialListMock.mockResolvedValueOnce([]);

    render(<ProvidersSection />);

    const heading = await screen.findByText("No provider keys");
    const state = heading.closest(".settings-group__state");
    expect(state?.children).toHaveLength(2);
    expect(state).toHaveTextContent("Add one to enable chat.");
    expect(screen.queryByText("Provider key required")).not.toBeInTheDocument();
    expect(state?.querySelector("button")).toBeNull();
    expect(screen.getAllByRole("button", { name: "Add provider key" })).toHaveLength(1);
  });

  it("shows each key as a list row with its label, provider, default badge, status and remove button", async () => {
    credentialListMock.mockResolvedValueOnce([openAiKey]);

    render(<ProvidersSection />);

    const list = await screen.findByRole("list");
    const [row] = within(list).getAllByRole("listitem");
    expect(within(row).getByText("Work OpenAI")).toBeInTheDocument();
    expect(within(row).getByText("OpenAI")).toBeInTheDocument();
    expect(within(row).getByText("Default key")).toBeInTheDocument();
    expect(within(row).getByText("Ready")).toBeInTheDocument();
    expect(within(row).getByRole("button", { name: "Remove Work OpenAI" })).toBeInTheDocument();
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
  });

  it("offers Re-enter key only on the row that needs it", async () => {
    credentialListMock.mockResolvedValueOnce([
      openAiKey,
      { ...openAiKey, id: "cred-2", display_name: "openai_2", status: "needs_reentry", is_default: false },
    ]);

    render(<ProvidersSection />);

    const [readyRow, stranded] = within(await screen.findByRole("list")).getAllByRole("listitem");
    expect(within(readyRow).queryByRole("button", { name: "Re-enter key" })).not.toBeInTheDocument();
    expect(within(stranded).getByRole("button", { name: "Re-enter key" })).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "Re-enter key" })).toHaveLength(1);
  });

  it("stores a key for the chosen provider and reports that chat can now run", async () => {
    credentialListMock
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([{ ...openAiKey, provider: "anthropic", display_name: "anthropic_1" }]);
    credentialCreateMock.mockResolvedValueOnce({
      ...openAiKey,
      provider: "anthropic",
      display_name: "anthropic_1",
    });
    const onCredentialsChanged = vi.fn();
    const user = userEvent.setup();

    render(
      <>
        <ProvidersSection onCredentialsChanged={onCredentialsChanged} />
        <Toaster />
      </>,
    );
    await user.click(await screen.findByRole("button", { name: "Add provider key" }));

    await user.click(screen.getByRole("radio", { name: "Anthropic" }));
    await user.type(screen.getByLabelText("API key"), "sk-secret-value");
    await user.click(screen.getByRole("button", { name: "Save key" }));

    await waitFor(() =>
      expect(credentialCreateMock).toHaveBeenCalledWith("anthropic", "anthropic_1", "sk-secret-value", true),
    );
    expect(onCredentialsChanged).toHaveBeenCalledOnce();
    expect(await screen.findByRole("status")).toHaveTextContent("Added the provider key “anthropic_1”.");
    expect(await screen.findByText("anthropic_1")).toBeInTheDocument();
  });

  it("assigns unused provider-specific names without a label field", async () => {
    credentialListMock.mockResolvedValue([
      { ...openAiKey, provider: "openrouter", display_name: "openrouter_1" },
      { ...openAiKey, id: "cred-2", provider: "openrouter", display_name: "openrouter_3" },
    ]);
    credentialCreateMock.mockResolvedValue({ ...openAiKey, provider: "openrouter", display_name: "openrouter_2" });
    const user = userEvent.setup();
    render(<ProvidersSection />);
    await user.click(await screen.findByRole("button", { name: "Add provider key" }));
    expect(screen.queryByLabelText("Label")).not.toBeInTheDocument();
    expect(screen.getByText("Key name: openai_1")).toBeInTheDocument();
    await user.click(screen.getByRole("radio", { name: "OpenRouter" }));
    expect(screen.getByText("Key name: openrouter_2")).toBeInTheDocument();
    await user.type(screen.getByLabelText("API key"), "  sk-or-secret-value  ");
    await user.click(screen.getByRole("button", { name: "Save key" }));
    await waitFor(() =>
      expect(credentialCreateMock).toHaveBeenCalledWith("openrouter", "openrouter_2", "sk-or-secret-value", true),
    );
  });

  it.each(["reconciliation_required", "conflict"])(
    "refreshes the list after %s without repeating the save",
    async (code) => {
      credentialListMock.mockResolvedValueOnce([]).mockResolvedValueOnce([openAiKey]);
      credentialCreateMock.mockRejectedValueOnce({ version: 1, code, credentials: null });
      const user = userEvent.setup();
      render(<ProvidersSection />);
      await user.click(await screen.findByRole("button", { name: "Add provider key" }));
      await user.type(screen.getByLabelText("API key"), "sk-secret-value");
      await user.click(screen.getByRole("button", { name: "Save key" }));
      expect(await screen.findByText("Work OpenAI")).toBeInTheDocument();
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
      expect(screen.getByText("Review provider keys")).toBeInTheDocument();
      expect(credentialListMock).toHaveBeenCalledTimes(2);
      expect(credentialCreateMock).toHaveBeenCalledOnce();
    },
  );

  it("never renders a secret back, only the redacted projection", async () => {
    credentialListMock.mockResolvedValueOnce([openAiKey]);

    render(<ProvidersSection />);

    expect(await screen.findByText("Work OpenAI")).toBeInTheDocument();
    expect(screen.getByText("OpenAI")).toBeInTheDocument();
    expect(screen.getByText("Default key")).toBeInTheDocument();
    expect(screen.getByText("Ready")).toBeInTheDocument();
  });

  it("explains a needs-re-entry key rather than leaving it looking healthy", async () => {
    credentialListMock.mockResolvedValueOnce([{ ...openAiKey, status: "needs_reentry", is_default: false }]);

    render(<ProvidersSection />);

    expect(await screen.findByText("Needs re-entry")).toBeInTheDocument();
    expect(screen.getByText(/Re-enter this legacy key/)).toBeInTheDocument();
  });

  it("turns a typed backend failure into a sentence instead of showing its code", async () => {
    credentialListMock.mockRejectedValueOnce({ version: 1, code: "service_unavailable", credentials: null });

    render(<ProvidersSection />);

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Provider keys could not be saved or retrieved. Try again.");
    expect(alert).not.toHaveTextContent("service_unavailable");
  });

  it("offers OpenRouter and stores its key under that provider, not OpenAI", async () => {
    credentialListMock
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([{ ...openAiKey, provider: "openrouter", display_name: "openrouter_1" }]);
    credentialCreateMock.mockResolvedValueOnce({
      ...openAiKey,
      provider: "openrouter",
      display_name: "openrouter_1",
    });
    const user = userEvent.setup();

    render(<ProvidersSection />);
    await user.click(await screen.findByRole("button", { name: "Add provider key" }));

    await user.click(screen.getByRole("radio", { name: "OpenRouter" }));
    expect(screen.getByRole("switch", { name: "Default key for this provider" })).toBeChecked();
    await user.type(screen.getByLabelText("API key"), "sk-or-secret-value");
    await user.click(screen.getByRole("button", { name: "Save key" }));

    await waitFor(() =>
      expect(credentialCreateMock).toHaveBeenCalledWith("openrouter", "openrouter_1", "sk-or-secret-value", true),
    );
    expect(await screen.findByText("openrouter_1")).toBeInTheDocument();
  });

  it("shows the selected provider's key shape so a key cannot be pasted into the wrong row unnoticed", async () => {
    credentialListMock.mockResolvedValueOnce([]);
    const user = userEvent.setup();

    render(<ProvidersSection />);
    await user.click(await screen.findByRole("button", { name: "Add provider key" }));

    expect(screen.getByText(/Starts with sk-\./)).toBeInTheDocument();

    await user.click(screen.getByRole("radio", { name: "OpenRouter" }));

    expect(screen.getByText(/Starts with sk-or-/)).toBeInTheDocument();
  });

  it("discards a typed key immediately when the add dialog is cancelled", async () => {
    credentialListMock.mockResolvedValueOnce([]);
    const user = userEvent.setup();

    render(<ProvidersSection />);
    await user.click(await screen.findByRole("button", { name: "Add provider key" }));
    await user.type(screen.getByLabelText("API key"), "sk-secret-that-must-be-discarded");
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    await user.click(screen.getByRole("button", { name: "Add provider key" }));

    expect(screen.getByLabelText("API key")).toHaveValue("");
  });

  // The status tells the user to paste the key again. Adding it again under the
  // same label collides with the workspace's uniqueness rule, so re-entry has to
  // act on the row itself.
  it("lets an unusable key be re-entered in place instead of added a second time", async () => {
    const stranded = {
      ...openAiKey,
      provider: "openrouter" as const,
      display_name: "Team OpenRouter",
      status: "needs_reentry" as const,
    };
    credentialListMock
      .mockResolvedValueOnce([stranded])
      .mockResolvedValueOnce([{ ...stranded, status: "ready" as const }]);
    credentialReplaceMock.mockResolvedValueOnce({ ...stranded, status: "ready" as const });
    const onCredentialsChanged = vi.fn();
    const user = userEvent.setup();

    render(
      <>
        <ProvidersSection onCredentialsChanged={onCredentialsChanged} />
        <Toaster />
      </>,
    );

    await user.click(await screen.findByRole("button", { name: "Re-enter key" }));
    const dialog = screen.getByRole("dialog", { name: "Re-enter provider key" });
    expect(dialog).toHaveTextContent("OpenRouter");
    expect(dialog).toHaveTextContent("Team OpenRouter");
    expect(screen.queryByRole("radio", { name: "OpenAI" }), "the provider belongs to the row").not.toBeInTheDocument();

    await user.type(screen.getByLabelText("API key"), "sk-or-fresh-value");
    await user.click(screen.getByRole("button", { name: "Save key" }));

    await waitFor(() => expect(credentialReplaceMock).toHaveBeenCalledWith("cred-1", "sk-or-fresh-value"));
    expect(credentialCreateMock, "re-entry must not add a second row").not.toHaveBeenCalled();
    expect(onCredentialsChanged).toHaveBeenCalledOnce();
    expect(await screen.findByRole("status")).toHaveTextContent("is stored securely and ready to use");
  });

  it("refreshes an unconfirmed re-entry and removes the pasted secret", async () => {
    credentialListMock.mockResolvedValue([{ ...openAiKey, status: "needs_reentry" }]);
    credentialReplaceMock.mockRejectedValueOnce({ version: 1, code: "reconciliation_required", credentials: null });
    const user = userEvent.setup();
    render(<ProvidersSection />);
    await user.click(await screen.findByRole("button", { name: "Re-enter key" }));
    await user.type(screen.getByLabelText("API key"), "sk-secret-value");
    await user.click(screen.getByRole("button", { name: "Save key" }));
    await waitFor(() => expect(credentialListMock).toHaveBeenCalledTimes(2));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.getByText("Review provider keys")).toBeInTheDocument();
    expect(credentialReplaceMock).toHaveBeenCalledOnce();
  });

  it("offers no re-entry on a key that is already usable", async () => {
    credentialListMock.mockResolvedValueOnce([openAiKey]);

    render(<ProvidersSection />);

    expect(await screen.findByText("Ready")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Re-enter key" })).not.toBeInTheDocument();
  });

  it("discards a typed key immediately when re-entry is cancelled", async () => {
    const stranded = { ...openAiKey, status: "needs_reentry" as const };
    credentialListMock.mockResolvedValueOnce([stranded]);
    const user = userEvent.setup();

    render(<ProvidersSection />);
    await user.click(await screen.findByRole("button", { name: "Re-enter key" }));
    await user.type(screen.getByLabelText("API key"), "sk-secret-that-must-be-discarded");
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    await user.click(screen.getByRole("button", { name: "Re-enter key" }));

    expect(screen.getByLabelText("API key")).toHaveValue("");
  });

  it("keeps a rejected key on screen with the provider's own reason", async () => {
    credentialListMock.mockResolvedValueOnce([]);
    credentialCreateMock.mockRejectedValueOnce({ version: 1, code: "invalid_credential", credentials: null });
    const user = userEvent.setup();

    render(<ProvidersSection />);
    await user.click(await screen.findByRole("button", { name: "Add provider key" }));
    await user.type(screen.getByLabelText("API key"), "nope");
    await user.click(screen.getByRole("button", { name: "Save key" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "The provider rejected the key. Check the key and re-enter it.",
    );
    expect(screen.getByRole("dialog", { name: "Add provider key" })).toBeInTheDocument();
  });

  it("removes a key only after the reader confirms, for every client", async () => {
    const spare = { ...openAiKey, id: "cred-2", display_name: "openai_2", is_default: false };
    credentialListMock.mockResolvedValueOnce([openAiKey, spare]).mockResolvedValueOnce([openAiKey]);
    credentialDeleteMock.mockResolvedValueOnce(undefined);
    const onCredentialsChanged = vi.fn();
    const user = userEvent.setup();

    render(
      <>
        <ProvidersSection onCredentialsChanged={onCredentialsChanged} />
        <Toaster />
      </>,
    );
    await user.click(await screen.findByRole("button", { name: "Remove openai_2" }));

    const dialog = screen.getByRole("alertdialog", { name: "Remove “openai_2”?" });
    expect(dialog).toHaveTextContent("every signed-in xFalcon client");
    // "Work OpenAI" stays ready, so chat does not stop.
    expect(dialog).toHaveTextContent("Chat with OpenAI keeps working with your other OpenAI key.");
    expect(credentialDeleteMock).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Remove key" }));

    await waitFor(() => expect(credentialDeleteMock).toHaveBeenCalledWith("cred-2"));
    expect(await screen.findByRole("status")).toHaveTextContent("Removed the provider key “openai_2”.");
    await waitFor(() => expect(screen.queryByText("openai_2")).not.toBeInTheDocument());
    expect(screen.getByText("Work OpenAI")).toBeInTheDocument();
    expect(onCredentialsChanged).toHaveBeenCalledOnce();
  });

  it("keeps the key when the reader backs out", async () => {
    credentialListMock.mockResolvedValueOnce([openAiKey]);
    const user = userEvent.setup();

    render(<ProvidersSection />);
    await user.click(await screen.findByRole("button", { name: "Remove Work OpenAI" }));
    await user.click(screen.getByRole("button", { name: "Cancel" }));

    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(credentialDeleteMock).not.toHaveBeenCalled();
    expect(screen.getByText("Work OpenAI")).toBeInTheDocument();
  });

  it("keeps the confirmation open with the reason when removal fails", async () => {
    credentialListMock.mockResolvedValue([openAiKey]);
    credentialDeleteMock.mockRejectedValueOnce({ version: 1, code: "service_unavailable", credentials: null });
    const onCredentialsChanged = vi.fn();
    const user = userEvent.setup();

    render(<ProvidersSection onCredentialsChanged={onCredentialsChanged} />);
    await user.click(await screen.findByRole("button", { name: "Remove Work OpenAI" }));
    await user.click(screen.getByRole("button", { name: "Remove key" }));

    const dialog = await screen.findByRole("alertdialog", { name: "Remove “Work OpenAI”?" });
    expect(dialog).toHaveTextContent("The action did not finish");
    // The last OpenAI key: removing it stops chat with OpenAI.
    expect(dialog).toHaveTextContent("Chat with OpenAI stops until another OpenAI key is added.");
    expect(onCredentialsChanged).not.toHaveBeenCalled();
    // The rows behind the dialog are re-read after a failure.
    await waitFor(() => expect(credentialListMock).toHaveBeenCalledTimes(2));
  });
});
