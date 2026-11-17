import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CustomModelDiscoveryResult, CustomModelView, SessionProjection } from "../../lib/generated/contracts";
import { resetAuthSessionForTests, setAuthSession } from "../auth/authStore";
import { CustomModelsGroup } from "./CustomModelsGroup";

const discoverCustomModelsMock = vi.hoisted(() => vi.fn());
const listCustomModelsMock = vi.hoisted(() => vi.fn());
const createCustomModelMock = vi.hoisted(() => vi.fn());
const updateCustomModelMock = vi.hoisted(() => vi.fn());
const deleteCustomModelMock = vi.hoisted(() => vi.fn());

vi.mock("../models/modelNative", () => ({
  listCustomModels: listCustomModelsMock,
  discoverCustomModels: discoverCustomModelsMock,
  createCustomModel: createCustomModelMock,
  updateCustomModel: updateCustomModelMock,
  deleteCustomModel: deleteCustomModelMock,
}));

const GATEWAY_URL = "https://llm-gateway.iseyon.workers.dev/v1";

const privateModel: CustomModelView = {
  version: 1,
  id: "018f0f45-6d2d-7b41-a741-a488b456805a",
  model_id: "custom/018f0f45-6d2d-7b41-a741-a488b456805a",
  display_name: "Private Qwen",
  base_url: "https://models.example.com/v1",
  upstream_model_id: "qwen-max",
  context_length: 32768,
  max_output_tokens: 4096,
  supports_tools: true,
  supports_vision: false,
  sampling: { temperature: null, top_p: null },
  is_enabled: true,
  is_active: true,
};

const discovered: CustomModelDiscoveryResult = {
  version: 1,
  existing_models: [],
  unavailable_reason: null,
  models: [
    {
      upstream_model_id: privateModel.upstream_model_id,
      display_name: privateModel.display_name,
      context_length: privateModel.context_length,
      max_output_tokens: privateModel.max_output_tokens,
      supports_tools: true,
      supports_vision: false,
      sampling: { temperature: 0.7, top_p: 0.9 },
      supported_sampling_parameters: ["temperature", "top_p"],
    },
  ],
};

async function openAdd(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: "Add private model" }));
  const dialog = screen.getByRole("dialog", { name: "Add private model" });
  await user.type(within(dialog).getByLabelText(/API key/), "secret-value");
  await user.click(within(dialog).getByRole("button", { name: "Next" }));
  await within(dialog).findByLabelText(/Context length/);
  return dialog;
}

function authenticated(userId: string): SessionProjection {
  return {
    status: "authenticated",
    version: 1,
    expires_at_ms: 1_000,
    identity: {
      version: 1,
      user_id: userId,
      tenant_id: "tenant",
      display_name: userId,
      email: null,
    },
  };
}

beforeEach(() => {
  listCustomModelsMock.mockResolvedValue([]);
  discoverCustomModelsMock.mockResolvedValue(discovered);
});

afterEach(() => {
  cleanup();
  resetAuthSessionForTests();
  listCustomModelsMock.mockReset();
  discoverCustomModelsMock.mockReset();
  createCustomModelMock.mockReset();
  updateCustomModelMock.mockReset();
  deleteCustomModelMock.mockReset();
});

describe("CustomModelsGroup", () => {
  it("keeps an incomplete saved model visible and lets the user repair its output limit", async () => {
    const incomplete = { ...privateModel, max_output_tokens: null };
    listCustomModelsMock.mockResolvedValueOnce([incomplete]);
    updateCustomModelMock.mockResolvedValueOnce(privateModel);
    const user = userEvent.setup();
    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);

    await screen.findByText("Private Qwen");
    expect(screen.getByText("Needs settings")).toBeVisible();
    expect(screen.getByText("Set an output limit to use this model.")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Edit" }));
    const dialog = screen.getByRole("dialog", { name: "Edit “Private Qwen”" });
    const output = within(dialog).getByLabelText(/Maximum output tokens/);
    expect(output).toHaveValue(null);
    expect(within(dialog).getByRole("button", { name: "Save changes" })).toBeDisabled();
    await user.type(output, "4096");
    await user.click(within(dialog).getByRole("button", { name: "Test and save changes" }));

    await waitFor(() =>
      expect(updateCustomModelMock).toHaveBeenCalledWith(privateModel.id, { max_output_tokens: 4096 }),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("lets the user remove an incomplete saved model", async () => {
    listCustomModelsMock.mockResolvedValueOnce([{ ...privateModel, max_output_tokens: null }]);
    deleteCustomModelMock.mockResolvedValueOnce(undefined);
    const user = userEvent.setup();
    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);

    await screen.findByText("Needs settings");
    await user.click(screen.getByRole("button", { name: "Remove" }));
    await user.click(screen.getByRole("button", { name: "Remove model" }));
    await waitFor(() => expect(deleteCustomModelMock).toHaveBeenCalledWith(privateModel.id));
  });

  it("blocks an existing endpoint model before save regardless of display name or manual entry", async () => {
    discoverCustomModelsMock.mockResolvedValueOnce({
      ...discovered,
      existing_models: [{ upstream_model_id: privateModel.upstream_model_id, display_name: "Company coding model" }],
    });
    const user = userEvent.setup();
    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
    const dialog = await openAdd(user);
    expect(within(dialog).getByText(/already added as “Company coding model”/)).toBeVisible();
    await user.clear(within(dialog).getByLabelText(/Display name/));
    await user.type(within(dialog).getByLabelText(/Display name/), "A different name");
    expect(within(dialog).getByRole("button", { name: "Test and add model" })).toBeDisabled();
    await user.click(within(dialog).getByRole("button", { name: "Enter a model name instead" }));
    await user.type(within(dialog).getByLabelText(/Model name/), privateModel.upstream_model_id);
    expect(within(dialog).getByText(/already added as “Company coding model”/)).toBeVisible();
    expect(createCustomModelMock).not.toHaveBeenCalled();
  });

  it("rechecks uniqueness for a different API key in the same modal", async () => {
    discoverCustomModelsMock.mockResolvedValueOnce({
      ...discovered,
      existing_models: [{ upstream_model_id: privateModel.upstream_model_id, display_name: "Existing model" }],
    });
    const user = userEvent.setup();
    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
    const dialog = await openAdd(user);
    await user.click(within(dialog).getByRole("button", { name: "Back" }));
    await user.clear(within(dialog).getByLabelText(/API key/));
    await user.type(within(dialog).getByLabelText(/API key/), "another-key");
    await user.click(within(dialog).getByRole("button", { name: "Next" }));
    await within(dialog).findByLabelText(/Context length/);
    expect(within(dialog).queryByText(/already added as/)).not.toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Test and add model" })).toBeEnabled();
    expect(discoverCustomModelsMock).toHaveBeenLastCalledWith({ base_url: GATEWAY_URL, secret: "another-key" });
  });

  it("prefills labeled suggestions when a listed model supplies no metadata", async () => {
    discoverCustomModelsMock.mockResolvedValueOnce({
      ...discovered,
      models: [
        {
          ...discovered.models[0],
          upstream_model_id: "ollama/qwen3.6:35b-a3b-coding-nvfp4",
          context_length: null,
          max_output_tokens: null,
          sampling: { temperature: null, top_p: null },
        },
      ],
    });
    const user = userEvent.setup();
    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
    const dialog = await openAdd(user);
    expect(within(dialog).getByLabelText(/Context length/)).toHaveValue(32768);
    expect(within(dialog).getByLabelText(/Maximum output tokens/)).toHaveValue(4096);
    expect(within(dialog).getByLabelText(/Temperature/)).toHaveValue(0.7);
    expect(within(dialog).getByLabelText(/Top P/)).toHaveValue(0.9);
    expect(within(dialog).getAllByText(/Suggested by xFalcon/)).toHaveLength(4);
    expect(within(dialog).getByRole("button", { name: "Test and add model" })).toBeEnabled();
  });

  it("clears a save error when a token setting is corrected and keeps the form open", async () => {
    const reason =
      "context_length must leave room for model output and 16384 reserved tokens for xFalcon's instructions and tools.";
    createCustomModelMock.mockRejectedValueOnce({ version: 1, code: "failed", message: reason });
    const user = userEvent.setup();
    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
    const dialog = await openAdd(user);
    await user.click(within(dialog).getByRole("button", { name: "Test and add model" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("These limits leave no room for input.");
    const context = within(dialog).getByLabelText(/Context length/);
    await user.clear(context);
    await user.type(context, "65536");
    await user.tab();
    expect(within(dialog).queryByText(reason)).not.toBeInTheDocument();
    expect(within(dialog).queryByRole("button", { name: "Dismiss error" })).not.toBeInTheDocument();
    expect(context).toHaveValue(65536);
    expect(within(dialog).getByLabelText(/Maximum output tokens/)).toHaveValue(4096);
    await user.type(within(dialog).getByLabelText(/API key/), "corrected-key");
    expect(within(dialog).getByRole("button", { name: "Test and add model" })).toBeEnabled();
  });

  it("lets users dismiss a save error without losing their settings", async () => {
    const reason = "Could not check for existing private models. Try again.";
    createCustomModelMock.mockRejectedValueOnce({ version: 1, code: "failed", message: reason });
    const user = userEvent.setup();
    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
    const dialog = await openAdd(user);
    await user.click(within(dialog).getByRole("button", { name: "Test and add model" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(reason);
    await user.click(within(dialog).getByRole("button", { name: "Dismiss error" }));
    expect(within(dialog).queryByText(reason)).not.toBeInTheDocument();
    expect(within(dialog).getByLabelText(/Context length/)).toHaveValue(32768);
    expect(within(dialog).getByLabelText(/API key/)).toHaveValue("");
    expect(dialog).toBeInTheDocument();
  });

  it("checks the instruction reserve before saving and recovers when either limit changes", async () => {
    const user = userEvent.setup();
    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
    const dialog = await openAdd(user);
    const context = within(dialog).getByLabelText(/Context length/);
    const output = within(dialog).getByLabelText(/Maximum output tokens/);
    await user.clear(context);
    await user.type(context, "20480");
    await user.tab();
    expect(within(dialog).getByRole("button", { name: "Test and add model" })).toBeDisabled();
    expect(output).toHaveAttribute("aria-invalid", "true");
    expect(within(dialog).getByRole("alert")).toHaveTextContent(/4,095 output tokens or fewer/);
    expect(createCustomModelMock).not.toHaveBeenCalled();
    await user.clear(output);
    await user.type(output, "4095");
    await user.tab();
    expect(within(dialog).queryByRole("alert")).not.toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Test and add model" })).toBeEnabled();
    await user.clear(context);
    await user.type(context, "16384");
    await user.tab();
    expect(context).toHaveAttribute("aria-invalid", "true");
    expect(within(dialog).getByRole("alert")).toHaveTextContent(/at least 16,386 context tokens/);
    await user.clear(context);
    await user.type(context, "32768");
    await user.tab();
    expect(within(dialog).queryByRole("alert")).not.toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Test and add model" })).toBeEnabled();
  });

  it.each([
    { context: 20000, output: null, expectedContext: 20000, expectedOutput: 1808 },
    { context: null, output: 32768, expectedContext: 61440, expectedOutput: 32768 },
    { context: 500000, output: 450000, expectedContext: 500000, expectedOutput: 450000 },
  ])(
    "preserves reported limits and fits missing suggestions for $context/$output",
    async ({ context, output, expectedContext, expectedOutput }) => {
      discoverCustomModelsMock.mockResolvedValueOnce({
        ...discovered,
        models: [
          {
            ...discovered.models[0],
            context_length: context,
            max_output_tokens: output,
            sampling: { temperature: 0, top_p: 0 },
          },
        ],
      });
      const user = userEvent.setup();
      render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
      const dialog = await openAdd(user);
      expect(within(dialog).getByLabelText(/Context length/)).toHaveValue(expectedContext);
      expect(within(dialog).getByLabelText(/Maximum output tokens/)).toHaveValue(expectedOutput);
      expect(within(dialog).getByLabelText(/Temperature/)).toHaveValue(0);
      expect(within(dialog).getByLabelText(/Top P/)).toHaveValue(0);
      expect(within(dialog).getAllByText(/Provided by your model endpoint/)).toHaveLength(
        context !== null && output !== null ? 4 : 3,
      );
      expect(within(dialog).getByRole("button", { name: "Test and add model" })).toBeEnabled();
    },
  );

  it("shows an unusable reported context immediately without inflating the model's limit", async () => {
    discoverCustomModelsMock.mockResolvedValueOnce({
      ...discovered,
      models: [{ ...discovered.models[0], context_length: 8192 }],
    });
    const user = userEvent.setup();
    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
    const dialog = await openAdd(user);
    expect(within(dialog).getByLabelText(/Context length/)).toHaveValue(8192);
    expect(within(dialog).getByLabelText(/Context length/)).toHaveAttribute("aria-invalid", "true");
    expect(within(dialog).getByRole("alert")).toHaveTextContent(/at least 16,386 context tokens/);
    expect(within(dialog).getByRole("button", { name: "Test and add model" })).toBeDisabled();
    expect(createCustomModelMock).not.toHaveBeenCalled();
  });

  it.each([
    { context: 16385, output: 1, valid: false },
    { context: 144384, output: 128001, valid: false },
    { context: 144385, output: 128001, valid: true },
  ])("matches the backend at the exact $context/$output budget boundary", async ({ context, output, valid }) => {
    discoverCustomModelsMock.mockResolvedValueOnce({
      ...discovered,
      models: [{ ...discovered.models[0], context_length: context, max_output_tokens: output }],
    });
    const user = userEvent.setup();
    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
    const dialog = await openAdd(user);
    expect(within(dialog).getByLabelText(/Context length/)).toHaveValue(context);
    expect(within(dialog).getByLabelText(/Maximum output tokens/)).toHaveValue(output);
    const save = within(dialog).getByRole("button", { name: "Test and add model" });
    if (valid) {
      expect(save).toBeEnabled();
      expect(within(dialog).queryByRole("alert")).not.toBeInTheDocument();
    } else {
      expect(save).toBeDisabled();
      expect(within(dialog).getByRole("alert")).toBeVisible();
      expect(within(dialog).queryByText(/Use 0 output tokens/)).not.toBeInTheDocument();
    }
  });

  it("labels only prefilled values and preserves saved settings when editing", async () => {
    const user = userEvent.setup();
    listCustomModelsMock.mockResolvedValue([privateModel]);
    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
    let dialog = await openAdd(user);
    expect(within(dialog).getAllByText(/Provided by your model endpoint/)).toHaveLength(4);
    await user.clear(within(dialog).getByLabelText(/Temperature/));
    await user.type(within(dialog).getByLabelText(/Temperature/), "0.4");
    expect(within(dialog).getAllByText(/Provided by your model endpoint/)).toHaveLength(3);
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    await user.click(screen.getByRole("button", { name: "Edit" }));
    dialog = screen.getByRole("dialog", { name: "Edit “Private Qwen”" });
    expect(within(dialog).getByLabelText(/Context length/)).toHaveValue(privateModel.context_length);
    expect(within(dialog).getByLabelText(/Temperature/)).toHaveValue(null);
    expect(within(dialog).getByLabelText(/Top P/)).toHaveValue(null);
    expect(within(dialog).queryByText(/Provided by your model endpoint|Suggested by xFalcon/)).not.toBeInTheDocument();
  });

  it("guards rapid discovery and save clicks and discards a save result after switching accounts", async () => {
    let resolveDiscovery: ((result: CustomModelDiscoveryResult) => void) | undefined;
    let resolveSave: ((result: CustomModelView) => void) | undefined;
    discoverCustomModelsMock.mockReturnValueOnce(
      new Promise<CustomModelDiscoveryResult>((resolve) => {
        resolveDiscovery = resolve;
      }),
    );
    createCustomModelMock.mockReturnValueOnce(
      new Promise<CustomModelView>((resolve) => {
        resolveSave = resolve;
      }),
    );
    setAuthSession(authenticated("account-one"));
    const onCatalogChanged = vi.fn();
    const user = userEvent.setup();
    render(<CustomModelsGroup onCatalogChanged={onCatalogChanged} />);
    await user.click(screen.getByRole("button", { name: "Add private model" }));
    const dialog = screen.getByRole("dialog", { name: "Add private model" });
    await user.type(within(dialog).getByLabelText(/API key/), "secret-value");
    await user.dblClick(within(dialog).getByRole("button", { name: "Next" }));
    expect(discoverCustomModelsMock).toHaveBeenCalledTimes(1);
    expect(within(dialog).getByLabelText(/Endpoint URL/)).toBeDisabled();
    expect(within(dialog).getByLabelText(/API key/)).toBeDisabled();
    await act(async () => resolveDiscovery?.(discovered));
    await user.dblClick(within(dialog).getByRole("button", { name: "Test and add model" }));
    expect(createCustomModelMock).toHaveBeenCalledTimes(1);
    expect(within(dialog).getByLabelText(/Context length/)).toBeDisabled();
    expect(within(dialog).getByRole("button", { name: "Back" })).toBeDisabled();
    await act(async () => setAuthSession(authenticated("account-two")));
    await act(async () => resolveSave?.(privateModel));
    expect(dialog).not.toBeInTheDocument();
    expect(screen.queryByText("Private Qwen")).not.toBeInTheDocument();
    expect(onCatalogChanged).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Add private model" }));
    expect(
      screen.getByRole("dialog", { name: "Add private model" }).querySelector('input[type="password"]'),
    ).toHaveValue("");
  });

  it("retains the editor when the duplicate preflight fails before saving", async () => {
    const message = "Could not check for existing private models. Try again.";
    createCustomModelMock.mockRejectedValueOnce({ version: 1, code: "failed", message });
    const onCatalogChanged = vi.fn();
    const user = userEvent.setup();
    render(<CustomModelsGroup onCatalogChanged={onCatalogChanged} />);
    const dialog = await openAdd(user);
    await user.clear(within(dialog).getByLabelText(/Display name/));
    await user.type(within(dialog).getByLabelText(/Display name/), "My coding model");
    await user.click(within(dialog).getByRole("button", { name: "Test and add model" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(message);
    expect(dialog).toBeInTheDocument();
    expect(within(dialog).getByLabelText(/Display name/)).toHaveValue("My coding model");
    expect(within(dialog).getByLabelText(/Context length/)).toHaveValue(32768);
    expect(within(dialog).getByLabelText(/Maximum output tokens/)).toHaveValue(4096);
    expect(within(dialog).getByLabelText(/Temperature/)).toHaveValue(0.7);
    expect(within(dialog).getByLabelText(/Top P/)).toHaveValue(0.9);
    expect(within(dialog).getByLabelText(/API key/)).toHaveValue("");
    expect(onCatalogChanged).not.toHaveBeenCalled();
    expect(listCustomModelsMock).toHaveBeenCalledTimes(1);
  });

  it("retains an existing model's editor when its initial catalog read fails", async () => {
    listCustomModelsMock.mockResolvedValue([privateModel]);
    const message = "Could not check for existing private models. Try again.";
    updateCustomModelMock.mockRejectedValueOnce({ version: 1, code: "failed", message });
    const onCatalogChanged = vi.fn();
    const user = userEvent.setup();
    render(<CustomModelsGroup onCatalogChanged={onCatalogChanged} />);
    await user.click(await screen.findByRole("button", { name: "Edit" }));
    const dialog = screen.getByRole("dialog", { name: `Edit “${privateModel.display_name}”` });
    await user.clear(within(dialog).getByLabelText(/Display name/));
    await user.type(within(dialog).getByLabelText(/Display name/), "Updated coding model");
    await user.click(within(dialog).getByRole("button", { name: "Save changes" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(message);
    expect(dialog).toBeInTheDocument();
    expect(within(dialog).getByLabelText(/Display name/)).toHaveValue("Updated coding model");
    expect(within(dialog).getByLabelText(/Context length/)).toHaveValue(privateModel.context_length);
    expect(within(dialog).getByLabelText(/API key/)).toHaveValue("");
    expect(onCatalogChanged).not.toHaveBeenCalled();
    expect(listCustomModelsMock).toHaveBeenCalledTimes(1);
  });

  it("keeps the dialog open after a connection test fails and allows a corrected retry", async () => {
    let rejectCheck: ((cause: unknown) => void) | undefined;
    createCustomModelMock.mockReturnValueOnce(
      new Promise<CustomModelView>((_resolve, reject) => {
        rejectCheck = reject;
      }),
    );
    const onCatalogChanged = vi.fn();
    const user = userEvent.setup();
    render(<CustomModelsGroup onCatalogChanged={onCatalogChanged} />);
    const dialog = await openAdd(user);
    await user.click(within(dialog).getByRole("button", { name: "Test and add model" }));
    expect(within(dialog).getByRole("status")).toHaveTextContent("Checking a short greeting before saving the model.");
    expect(within(dialog).getByRole("button", { name: "Testing and saving…" })).toBeDisabled();
    expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeDisabled();
    await user.keyboard("{Escape}");
    expect(dialog).toBeInTheDocument();
    expect(onCatalogChanged).not.toHaveBeenCalled();

    const reason = "The provider connection test timed out. Check the API base URL and try again.";
    await act(async () => rejectCheck?.({ version: 1, code: "failed", message: reason }));
    const alert = await within(dialog).findByRole("alert");
    expect(alert).toHaveTextContent(reason);
    expect(alert).toHaveFocus();
    expect(within(dialog).getByLabelText(/API key/)).toHaveValue("");
    expect(within(dialog).getByLabelText(/Context length/)).toHaveValue(32768);
    expect(within(dialog).getByRole("button", { name: "Test and add model" })).toBeDisabled();
    expect(onCatalogChanged).not.toHaveBeenCalled();
    expect(listCustomModelsMock).toHaveBeenCalledTimes(1);

    createCustomModelMock.mockResolvedValueOnce(privateModel);
    await user.type(within(dialog).getByLabelText(/API key/), "corrected-key");
    await user.click(within(dialog).getByRole("button", { name: "Test and add model" }));
    await waitFor(() => expect(dialog).not.toBeInTheDocument());
    expect(createCustomModelMock).toHaveBeenLastCalledWith(expect.objectContaining({ secret: "corrected-key" }));
    expect(onCatalogChanged).toHaveBeenCalledTimes(1);
  });

  it("drops discovery from a previous account before any model details can render", async () => {
    let resolveOld: ((result: CustomModelDiscoveryResult) => void) | undefined;
    discoverCustomModelsMock.mockReturnValueOnce(
      new Promise<CustomModelDiscoveryResult>((resolve) => {
        resolveOld = resolve;
      }),
    );
    setAuthSession(authenticated("account-one"));
    const user = userEvent.setup();
    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: "Add private model" }));
    const dialog = screen.getByRole("dialog", { name: "Add private model" });
    await user.type(within(dialog).getByLabelText(/API key/), "secret-value");
    await user.click(within(dialog).getByRole("button", { name: "Next" }));
    setAuthSession(authenticated("account-two"));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    resolveOld?.(discovered);
    await Promise.resolve();
    expect(screen.queryByDisplayValue("Private Qwen")).not.toBeInTheDocument();
    expect(screen.queryByDisplayValue("secret-value")).not.toBeInTheDocument();
  });

  it("searches large catalogs so models after the initial visible options remain selectable", async () => {
    discoverCustomModelsMock.mockResolvedValueOnce({
      ...discovered,
      models: Array.from({ length: 100 }, (_, index) => ({
        ...discovered.models[0],
        upstream_model_id: `vendor/model-${index}`,
        display_name: `Model ${index}`,
      })),
    });
    createCustomModelMock.mockResolvedValueOnce(privateModel);
    const user = userEvent.setup();
    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
    const dialog = await openAdd(user);
    await user.type(within(dialog).getByLabelText("Search models"), "model-99");
    await user.click(within(dialog).getByRole("button", { name: /^Model,/ }));
    await user.click(screen.getByRole("menuitemradio", { name: /vendor\/model-99/ }));
    await user.click(within(dialog).getByRole("button", { name: "Test and add model" }));
    await waitFor(() =>
      expect(createCustomModelMock).toHaveBeenCalledWith(
        expect.objectContaining({ upstream_model_id: "vendor/model-99" }),
      ),
    );
  });

  it.each(["unsupported", "unreachable", "invalid_response", "empty"] as const)(
    "allows manual entry after discovery reports %s without inventing limits",
    async (reason) => {
      discoverCustomModelsMock.mockResolvedValueOnce({
        version: 1,
        models: [],
        existing_models: [],
        unavailable_reason: reason,
      });
      createCustomModelMock.mockResolvedValueOnce(privateModel);
      const user = userEvent.setup();
      render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
      const dialog = await openAdd(user);
      expect(within(dialog).getByLabelText(/Model name/)).toHaveFocus();
      expect(within(dialog).getByText(/ollama\/<model> or openai\/<model>/)).toBeVisible();
      expect(within(dialog).getByLabelText(/Context length/)).toHaveValue(32768);
      expect(within(dialog).getByLabelText(/Maximum output tokens/)).toHaveValue(4096);
      expect(within(dialog).getByRole("button", { name: "Test and add model" })).toBeDisabled();
      await user.type(within(dialog).getByLabelText(/Model name/), "vendor/private-model:v2");
      await user.clear(within(dialog).getByLabelText(/Context length/));
      await user.type(within(dialog).getByLabelText(/Context length/), "65536");
      await user.clear(within(dialog).getByLabelText(/Maximum output tokens/));
      await user.type(within(dialog).getByLabelText(/Maximum output tokens/), "8192");
      await user.click(within(dialog).getByRole("switch", { name: "Supports tool calling" }));
      await user.click(within(dialog).getByRole("button", { name: "Test and add model" }));
      await waitFor(() =>
        expect(createCustomModelMock).toHaveBeenCalledWith(
          expect.objectContaining({
            upstream_model_id: "vendor/private-model:v2",
            display_name: "vendor/private-model:v2",
            context_length: 65536,
            max_output_tokens: 8192,
            supports_tools: true,
            sampling: { temperature: 0.7, top_p: 0.9 },
          }),
        ),
      );
    },
  );

  it("chooses exact returned IDs and replaces metadata when choosing another model", async () => {
    discoverCustomModelsMock.mockResolvedValueOnce({
      ...discovered,
      models: [
        discovered.models[0],
        {
          ...discovered.models[0],
          upstream_model_id: "ollama/new-model:latest",
          display_name: "New model",
          context_length: null,
          max_output_tokens: null,
          supports_tools: null,
          supports_vision: null,
          sampling: { temperature: null, top_p: null },
          supported_sampling_parameters: null,
        },
      ],
    });
    const user = userEvent.setup();
    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
    const dialog = await openAdd(user);
    await user.click(within(dialog).getByRole("button", { name: /^Model/ }));
    await user.click(screen.getByRole("menuitemradio", { name: /Private Qwen/ }));
    expect(within(dialog).getByLabelText(/Context length/)).toHaveValue(32768);
    await user.click(within(dialog).getByRole("button", { name: /^Model/ }));
    await user.click(screen.getByRole("menuitemradio", { name: /ollama\/new-model:latest/ }));
    expect(within(dialog).getByLabelText(/Context length/)).toHaveValue(32768);
    expect(within(dialog).getByLabelText(/Temperature/)).toHaveValue(0.7);
    expect(within(dialog).getByRole("switch", { name: "Supports tool calling" })).not.toBeChecked();
    expect(within(dialog).getByRole("button", { name: "Test and add model" })).toBeEnabled();
  });

  it("does not send sampling values that the catalog reports as unsupported", async () => {
    discoverCustomModelsMock.mockResolvedValueOnce({
      ...discovered,
      models: [
        {
          ...discovered.models[0],
          supported_sampling_parameters: [],
        },
      ],
    });
    createCustomModelMock.mockResolvedValueOnce(privateModel);
    const user = userEvent.setup();
    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
    const dialog = await openAdd(user);
    expect(within(dialog).getByLabelText(/Temperature/)).toBeDisabled();
    expect(within(dialog).getByLabelText(/Temperature/)).toHaveValue(null);
    await user.click(within(dialog).getByRole("button", { name: "Test and add model" }));
    await waitFor(() =>
      expect(createCustomModelMock).toHaveBeenCalledWith(
        expect.objectContaining({
          sampling: { temperature: null, top_p: null },
        }),
      ),
    );
  });

  it("respects unsupported sampling when a listed model ID is entered manually", async () => {
    discoverCustomModelsMock.mockResolvedValueOnce({
      ...discovered,
      models: [{ ...discovered.models[0], supported_sampling_parameters: [] }],
    });
    createCustomModelMock.mockResolvedValueOnce(privateModel);
    const user = userEvent.setup();
    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
    const dialog = await openAdd(user);
    await user.click(within(dialog).getByRole("button", { name: "Enter a model name instead" }));
    const temperature = within(dialog).getByLabelText(/Temperature/);
    await user.clear(temperature);
    await user.type(temperature, "3");
    await user.type(within(dialog).getByLabelText(/Model name/), ` ${privateModel.upstream_model_id} `);
    expect(temperature).toBeDisabled();
    expect(temperature).toHaveValue(null);
    expect(within(dialog).getByLabelText(/Top P/)).toBeDisabled();
    expect(within(dialog).getByRole("button", { name: "Test and add model" })).toBeEnabled();
    await user.click(within(dialog).getByRole("button", { name: "Test and add model" }));
    await waitFor(() =>
      expect(createCustomModelMock).toHaveBeenCalledWith(
        expect.objectContaining({
          upstream_model_id: privateModel.upstream_model_id,
          sampling: { temperature: null, top_p: null },
        }),
      ),
    );
  });

  it("shows native key rejection without echoing the secret and clears the submitted key", async () => {
    discoverCustomModelsMock.mockRejectedValueOnce({
      version: 1,
      code: "failed",
      message: "The provider rejected this API key.",
    });
    const user = userEvent.setup();
    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: "Add private model" }));
    const dialog = screen.getByRole("dialog", { name: "Add private model" });
    await user.type(within(dialog).getByLabelText(/API key/), "secret-value");
    await user.click(within(dialog).getByRole("button", { name: "Next" }));
    const error = await within(dialog).findByRole("alert");
    expect(error).toHaveTextContent("The provider rejected this API key.");
    expect(error).toHaveFocus();
    expect(within(dialog).getByLabelText(/API key/)).toHaveValue("");
    expect(createCustomModelMock).not.toHaveBeenCalled();
  });

  it("ignores a late discovery result after cancel and reopen", async () => {
    let resolveOld: ((result: CustomModelDiscoveryResult) => void) | undefined;
    discoverCustomModelsMock.mockReturnValueOnce(
      new Promise<CustomModelDiscoveryResult>((resolve) => {
        resolveOld = resolve;
      }),
    );
    const user = userEvent.setup();
    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: "Add private model" }));
    let dialog = screen.getByRole("dialog", { name: "Add private model" });
    await user.type(within(dialog).getByLabelText(/API key/), "secret-value");
    await user.click(within(dialog).getByRole("button", { name: "Next" }));
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    await user.click(screen.getByRole("button", { name: "Add private model" }));
    dialog = screen.getByRole("dialog", { name: "Add private model" });
    resolveOld?.(discovered);
    await Promise.resolve();
    expect(within(dialog).getByLabelText(/Endpoint URL/)).toBeVisible();
    expect(within(dialog).getByLabelText(/API key/)).toHaveValue("");
    expect(within(dialog).queryByLabelText(/Context length/)).not.toBeInTheDocument();
  });

  it("rejects output limits that leave no prompt space and out-of-range sampling", async () => {
    const user = userEvent.setup();
    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
    const dialog = await openAdd(user);
    const output = within(dialog).getByLabelText(/Maximum output tokens/);
    await user.clear(output);
    await user.type(output, "32768");
    await user.tab();
    expect(within(dialog).getByRole("alert")).toHaveTextContent("must be smaller than the context length");
    expect(within(dialog).getByRole("button", { name: "Test and add model" })).toBeDisabled();
    await user.clear(output);
    await user.type(output, "4096");
    const temperature = within(dialog).getByLabelText(/Temperature/);
    await user.clear(temperature);
    await user.type(temperature, "3");
    await user.tab();
    expect(within(dialog).getByRole("alert")).toHaveTextContent("from 0 to 2");
    expect(within(dialog).getByRole("button", { name: "Test and add model" })).toBeDisabled();
  });

  it("clears saved sampling overrides with a full replacement while retaining the stored key", async () => {
    listCustomModelsMock.mockResolvedValue([{ ...privateModel, sampling: { temperature: 0.6, top_p: 0.95 } }]);
    updateCustomModelMock.mockResolvedValueOnce(privateModel);
    const user = userEvent.setup();
    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
    await user.click(await screen.findByRole("button", { name: "Edit" }));
    const dialog = screen.getByRole("dialog", { name: `Edit “${privateModel.display_name}”` });
    await user.clear(within(dialog).getByLabelText(/Temperature/));
    await user.clear(within(dialog).getByLabelText(/Top P/));
    await user.click(within(dialog).getByRole("button", { name: "Test and save changes" }));
    await waitFor(() =>
      expect(updateCustomModelMock).toHaveBeenCalledWith(privateModel.id, {
        sampling: { temperature: null, top_p: null },
      }),
    );
  });

  it("starts with URL and key, discovers exact model details, and only saves after review", async () => {
    createCustomModelMock.mockResolvedValueOnce(privateModel);
    const onCatalogChanged = vi.fn();
    const user = userEvent.setup();
    render(<CustomModelsGroup onCatalogChanged={onCatalogChanged} />);
    await user.click(screen.getByRole("button", { name: "Add private model" }));
    const dialog = screen.getByRole("dialog", { name: "Add private model" });
    expect(within(dialog).getByLabelText(/Endpoint URL/)).toHaveValue(GATEWAY_URL);
    expect(within(dialog).queryByLabelText(/Context length/)).not.toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Next" })).toBeDisabled();
    await user.type(within(dialog).getByLabelText(/API key/), "secret-value");
    await user.click(within(dialog).getByRole("button", { name: "Next" }));
    await within(dialog).findByLabelText(/Context length/);
    expect(discoverCustomModelsMock).toHaveBeenCalledWith({ base_url: GATEWAY_URL, secret: "secret-value" });
    expect(createCustomModelMock).not.toHaveBeenCalled();
    expect(within(dialog).getByLabelText(/Display name/)).toHaveValue("Private Qwen");
    expect(within(dialog).getByLabelText(/Context length/)).toHaveValue(32768);
    expect(within(dialog).getByLabelText(/Maximum output tokens/)).toHaveValue(4096);
    expect(within(dialog).getByLabelText(/Temperature/)).toHaveValue(0.7);
    if (within(dialog).queryByRole("button", { name: "Next" })) {
      await user.click(within(dialog).getByRole("button", { name: "Next" }));
      await within(dialog).findByLabelText(/Context length/);
    }
    await user.click(within(dialog).getByRole("button", { name: "Test and add model" }));
    await waitFor(() =>
      expect(createCustomModelMock).toHaveBeenCalledWith({
        base_url: GATEWAY_URL,
        secret: "secret-value",
        upstream_model_id: "qwen-max",
        display_name: "Private Qwen",
        context_length: 32768,
        max_output_tokens: 4096,
        supports_tools: true,
        supports_vision: false,
        sampling: { temperature: 0.7, top_p: 0.9 },
      }),
    );
    expect(onCatalogChanged).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(listCustomModelsMock).toHaveBeenCalledTimes(2));
    expect(screen.queryByDisplayValue("secret-value")).not.toBeInTheDocument();
  });

  it("shows an allowlisted backend validation reason", async () => {
    createCustomModelMock.mockRejectedValueOnce({
      version: 1,
      code: "failed",
      message: "base_url host is not permitted on this deployment",
    });
    const user = userEvent.setup();

    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: "Add private model" }));
    const dialog = screen.getByRole("dialog", { name: "Add private model" });
    await user.type(within(dialog).getByLabelText(/API key/), "secret-value");
    if (within(dialog).queryByRole("button", { name: "Next" })) {
      await user.click(within(dialog).getByRole("button", { name: "Next" }));
      await within(dialog).findByLabelText(/Context length/);
    }
    await user.click(within(dialog).getByRole("button", { name: "Test and add model" }));

    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "base_url host is not permitted on this deployment",
    );
    expect(within(dialog).getByLabelText(/API key/)).toHaveValue("");
  });

  it("accepts http, localhost and IP endpoints", async () => {
    const user = userEvent.setup();
    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: "Add private model" }));
    const dialog = screen.getByRole("dialog", { name: "Add private model" });
    // `[` is a key-descriptor opener in user.type, so the IPv6 case is typed as `[[`.
    for (const { typed, expected } of [
      { typed: "http://localhost:8000/v1", expected: "http://localhost:8000/v1" },
      { typed: "http://127.0.0.1:11434/v1", expected: "http://127.0.0.1:11434/v1" },
      { typed: "http://192.168.1.20:8000/v1", expected: "http://192.168.1.20:8000/v1" },
      { typed: "http://[[::1]:8080/v1", expected: "http://[::1]:8080/v1" },
      { typed: "http://localhost:11434", expected: "http://localhost:11434" },
      { typed: "https://models.internal/openai/v1", expected: "https://models.internal/openai/v1" },
    ]) {
      await user.clear(within(dialog).getByLabelText(/Endpoint URL/));
      await user.type(within(dialog).getByLabelText(/Endpoint URL/), typed);
      await user.type(within(dialog).getByLabelText(/API key/), "secret-value");
      expect(within(dialog).getByLabelText(/Endpoint URL/)).toHaveValue(expected);
      expect(within(dialog).queryByText(/starts with http/)).not.toBeInTheDocument();
      expect(within(dialog).getByRole("button", { name: "Next" })).toBeEnabled();
      await user.clear(within(dialog).getByLabelText(/API key/));
    }
    expect(discoverCustomModelsMock).not.toHaveBeenCalled();
  });

  it("rejects endpoints that are not http or https", async () => {
    const user = userEvent.setup();
    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: "Add private model" }));
    const dialog = screen.getByRole("dialog", { name: "Add private model" });
    await user.type(within(dialog).getByLabelText(/API key/), "secret-value");
    for (const baseUrl of [
      "ftp://models.example.com/v1",
      "127.0.0.1:8000/v1",
      "http://user:pw@localhost:8000/v1",
      "http://localhost:8000/v1?x=1",
      "http://localhost:0/v1",
      "http://h/v1?",
      "http://h/v1#",
      "http://h/v1/../x",
      "http://h/v1/%2e%2e/x",
      "http://h\\v1",
      "http://bücher.example/v1",
      "http://127.1/v1",
      "http://010.0.0.1/v1",
      "http://0x7f.0.0.1/v1",
      "http://2130706433/v1",
    ]) {
      await user.clear(within(dialog).getByLabelText(/Endpoint URL/));
      await user.type(within(dialog).getByLabelText(/Endpoint URL/), baseUrl);
      await user.tab();
      expect(await within(dialog).findByText(/starts with http:\/\/ or https:\/\//)).toBeVisible();
      expect(within(dialog).getByRole("button", { name: "Next" })).toBeDisabled();
    }
    expect(discoverCustomModelsMock).not.toHaveBeenCalled();
  });

  it("shows the allowlisted API base URL refusal from the backend", async () => {
    createCustomModelMock.mockRejectedValueOnce({
      version: 1,
      code: "failed",
      message: "Enter a valid API base URL, such as http://localhost:8000/v1.",
    });
    const user = userEvent.setup();

    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: "Add private model" }));
    const dialog = screen.getByRole("dialog", { name: "Add private model" });
    await user.type(within(dialog).getByLabelText(/API key/), "secret-value");
    if (within(dialog).queryByRole("button", { name: "Next" })) {
      await user.click(within(dialog).getByRole("button", { name: "Next" }));
      await within(dialog).findByLabelText(/Context length/);
    }
    await user.click(within(dialog).getByRole("button", { name: "Test and add model" }));

    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "Enter a valid API base URL, such as http://localhost:8000/v1.",
    );
    expect(within(dialog).getByLabelText(/API key/)).toHaveValue("");
  });

  it.each([
    ["a 403 refusal", "Private models are disabled on this deployment."],
    ["a 409 conflict", "The private model limit was reached, or this model can no longer be changed."],
  ])("shows the distinct safe reason for %s", async (_case, message) => {
    createCustomModelMock.mockRejectedValueOnce({ version: 1, code: "failed", message });
    const user = userEvent.setup();

    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: "Add private model" }));
    const dialog = screen.getByRole("dialog", { name: "Add private model" });
    await user.type(within(dialog).getByLabelText(/API key/), "secret-value");
    if (within(dialog).queryByRole("button", { name: "Next" })) {
      await user.click(within(dialog).getByRole("button", { name: "Next" }));
      await within(dialog).findByLabelText(/Context length/);
    }
    await user.click(within(dialog).getByRole("button", { name: "Test and add model" }));

    expect(await within(dialog).findByRole("alert")).toHaveTextContent(message);
  });

  it("replaces an unsafe backend reason instead of echoing a key or full URL", async () => {
    createCustomModelMock.mockRejectedValueOnce({
      version: 1,
      code: "failed",
      message: `base_url ${privateModel.base_url} rejected secret-value`,
    });
    const user = userEvent.setup();

    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: "Add private model" }));
    const dialog = screen.getByRole("dialog", { name: "Add private model" });
    await user.clear(within(dialog).getByLabelText(/Endpoint URL/));
    await user.type(within(dialog).getByLabelText(/Endpoint URL/), privateModel.base_url);
    await user.type(within(dialog).getByLabelText(/API key/), "secret-value");
    if (within(dialog).queryByRole("button", { name: "Next" })) {
      await user.click(within(dialog).getByRole("button", { name: "Next" }));
      await within(dialog).findByLabelText(/Context length/);
    }
    await user.click(within(dialog).getByRole("button", { name: "Test and add model" }));

    const error = await within(dialog).findByRole("alert");
    expect(error).toHaveTextContent("The private model could not be added. Check the settings and try again.");
    expect(error).not.toHaveTextContent(privateModel.base_url);
    expect(error).not.toHaveTextContent("secret-value");
    expect(within(dialog).getByLabelText(/API key/)).toHaveValue("");
  });

  it("requires a new key for an endpoint change and sends only changed fields", async () => {
    listCustomModelsMock.mockResolvedValueOnce([privateModel]).mockResolvedValueOnce([privateModel]);
    updateCustomModelMock.mockResolvedValueOnce({
      ...privateModel,
      base_url: "https://new.example.com/v1",
    });
    const onCatalogChanged = vi.fn();
    const user = userEvent.setup();

    render(<CustomModelsGroup onCatalogChanged={onCatalogChanged} />);
    await user.click(await screen.findByRole("button", { name: "Edit" }));
    const dialog = screen.getByRole("dialog", { name: `Edit “${privateModel.display_name}”` });
    const key = within(dialog).getByLabelText(/API key/);
    expect(key).toHaveValue("");
    await user.clear(within(dialog).getByLabelText(/Endpoint URL/));
    await user.type(within(dialog).getByLabelText(/Endpoint URL/), "https://new.example.com/v1");
    expect(within(dialog).getByText(/new key is required because the endpoint changed/i)).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Test and save changes" })).toBeDisabled();

    await user.type(key, "replacement-key");
    await user.click(within(dialog).getByRole("button", { name: "Test and save changes" }));

    await waitFor(() =>
      expect(updateCustomModelMock).toHaveBeenCalledWith(privateModel.id, {
        base_url: "https://new.example.com/v1",
        secret: "replacement-key",
      }),
    );
    expect(onCatalogChanged).toHaveBeenCalledTimes(1);
  });

  it("removes a model and refreshes the catalog after confirmation", async () => {
    listCustomModelsMock.mockResolvedValueOnce([privateModel]).mockResolvedValueOnce([]);
    deleteCustomModelMock.mockResolvedValueOnce(undefined);
    const onCatalogChanged = vi.fn();
    const user = userEvent.setup();

    render(<CustomModelsGroup onCatalogChanged={onCatalogChanged} />);
    await user.click(await screen.findByRole("button", { name: "Remove" }));
    const dialog = screen.getByRole("alertdialog", { name: `Remove “${privateModel.display_name}”?` });
    await user.click(within(dialog).getByRole("button", { name: "Remove model" }));

    await waitFor(() => expect(deleteCustomModelMock).toHaveBeenCalledWith(privateModel.id));
    expect(onCatalogChanged).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(listCustomModelsMock).toHaveBeenCalledTimes(2));
  });

  it("drops a late private-model list from the prior account", async () => {
    let resolveOld: ((models: CustomModelView[]) => void) | undefined;
    listCustomModelsMock
      .mockReset()
      .mockReturnValueOnce(
        new Promise<CustomModelView[]>((resolve) => {
          resolveOld = resolve;
        }),
      )
      .mockResolvedValueOnce([{ ...privateModel, id: "account-two", display_name: "Account two model" }]);
    setAuthSession(authenticated("account-one"));

    render(<CustomModelsGroup onCatalogChanged={vi.fn()} />);
    await waitFor(() => expect(listCustomModelsMock).toHaveBeenCalledTimes(1));
    setAuthSession(authenticated("account-two"));
    await screen.findByText("Account two model");

    resolveOld?.([{ ...privateModel, id: "account-one", display_name: "Account one model" }]);
    await Promise.resolve();

    expect(screen.queryByText("Account one model")).not.toBeInTheDocument();
  });
});
