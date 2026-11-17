import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ChatModelCatalogView, SessionProjection } from "../../lib/generated/contracts";
import { resetAuthSessionForTests, setAuthSession } from "../auth/authStore";
import { invalidateModelCatalog, useModelCatalog } from "./useModelCatalog";

const loadModelCatalogMock = vi.hoisted(() => vi.fn());

vi.mock("./modelNative", () => ({
  loadModelCatalog: loadModelCatalogMock,
  saveDefaultModel: vi.fn(),
}));

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

function catalog(name: string, id: string): ChatModelCatalogView {
  return {
    version: 1,
    models: [
      {
        version: 1,
        id,
        provider: "openrouter",
        display_name: name,
        capabilities: { tools: true, image_input: false, reasoning_summary: true },
      },
    ],
    selected_model_id: id,
    selection_locked: false,
  };
}

function Probe() {
  const state = useModelCatalog(null);
  return (
    <>
      <div>{state.catalog?.models[0]?.display_name ?? "loading"}</div>
      <button type="button" onClick={state.refresh}>
        Refresh
      </button>
    </>
  );
}

function ConversationProbe({ conversationId }: { conversationId: string | null }) {
  const state = useModelCatalog(conversationId);
  return <div>{state.catalog?.models[0]?.display_name ?? "loading"}</div>;
}

function StateProbe({ conversationId }: { conversationId: string | null }) {
  const state = useModelCatalog(conversationId);
  return (
    <>
      <div>{state.catalog?.models[0]?.display_name ?? "no catalog"}</div>
      <div>{state.loading ? "loading" : "ready"}</div>
      <div>{state.error ?? "no error"}</div>
    </>
  );
}

function deferredCatalog() {
  let resolve!: (value: ChatModelCatalogView) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<ChatModelCatalogView>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

afterEach(() => {
  cleanup();
  // The cache is module state. Clear it so no test reads a catalog cached by an earlier test.
  invalidateModelCatalog();
  resetAuthSessionForTests();
  loadModelCatalogMock.mockReset();
});

describe("useModelCatalog account fencing", () => {
  it("drops a late catalog from the prior account and reloads for the current account", async () => {
    let resolveOld: ((value: ChatModelCatalogView) => void) | undefined;
    const oldRequest = new Promise<ChatModelCatalogView>((resolve) => {
      resolveOld = resolve;
    });
    loadModelCatalogMock.mockReturnValueOnce(oldRequest).mockResolvedValueOnce(catalog("Luna", "openai/gpt-5.6-luna"));
    setAuthSession(authenticated("account-one"));

    render(<Probe />);
    await waitFor(() => expect(loadModelCatalogMock).toHaveBeenCalledTimes(1));
    setAuthSession(authenticated("account-two"));
    await screen.findByText("Luna");

    resolveOld?.(catalog("Grok", "~x-ai/grok-latest"));
    await Promise.resolve();

    expect(screen.getByText("Luna")).toBeInTheDocument();
    expect(screen.queryByText("Grok")).not.toBeInTheDocument();
    expect(loadModelCatalogMock).toHaveBeenCalledTimes(2);
  });

  it("reloads the current account catalog on demand", async () => {
    loadModelCatalogMock
      .mockResolvedValueOnce(catalog("Sonnet", "~anthropic/claude-sonnet-latest"))
      .mockResolvedValueOnce(catalog("Private Qwen", "custom/018f0f45-6d2d-7b41-a741-a488b456805a"));
    const user = userEvent.setup();

    render(<Probe />);
    await screen.findByText("Sonnet");
    await user.click(screen.getByRole("button", { name: "Refresh" }));

    await screen.findByText("Private Qwen");
    expect(loadModelCatalogMock).toHaveBeenCalledTimes(2);
  });

  it("reloads when the conversation changes", async () => {
    loadModelCatalogMock
      .mockResolvedValueOnce(catalog("Sonnet", "~anthropic/claude-sonnet-latest"))
      .mockResolvedValueOnce(catalog("Private Qwen", "custom/018f0f45-6d2d-7b41-a741-a488b456805a"));

    const { rerender } = render(<ConversationProbe conversationId="conversation-one" />);
    await screen.findByText("Sonnet");
    rerender(<ConversationProbe conversationId="conversation-two" />);

    await screen.findByText("Private Qwen");
    expect(loadModelCatalogMock).toHaveBeenNthCalledWith(2, "conversation-two");
  });
});

describe("useModelCatalog invalidation", () => {
  it("reloads every mounted catalog once per invalidation", async () => {
    loadModelCatalogMock
      .mockResolvedValueOnce(catalog("Sonnet", "~anthropic/claude-sonnet-latest"))
      .mockResolvedValueOnce(catalog("Sonnet", "~anthropic/claude-sonnet-latest"))
      .mockResolvedValueOnce(catalog("Luna", "openai/gpt-5.6-luna"))
      .mockResolvedValueOnce(catalog("Luna", "openai/gpt-5.6-luna"));

    render(
      <>
        <Probe />
        <Probe />
      </>,
    );
    await waitFor(() => expect(screen.getAllByText("Sonnet")).toHaveLength(2));

    await act(async () => {
      invalidateModelCatalog();
    });

    await waitFor(() => expect(screen.getAllByText("Luna")).toHaveLength(2));
    expect(loadModelCatalogMock).toHaveBeenCalledTimes(4);
  });

  it("drops a catalog that finishes after a newer invalidation", async () => {
    let resolveStale!: (value: ChatModelCatalogView) => void;
    loadModelCatalogMock
      .mockReturnValueOnce(
        new Promise<ChatModelCatalogView>((resolve) => {
          resolveStale = resolve;
        }),
      )
      .mockResolvedValueOnce(catalog("Luna", "openai/gpt-5.6-luna"));
    render(<Probe />);
    await waitFor(() => expect(loadModelCatalogMock).toHaveBeenCalledTimes(1));

    await act(async () => {
      invalidateModelCatalog();
    });
    await screen.findByText("Luna");
    await act(async () => {
      resolveStale(catalog("Grok", "~x-ai/grok-latest"));
    });

    expect(screen.getByText("Luna")).toBeInTheDocument();
    expect(screen.queryByText("Grok")).not.toBeInTheDocument();
    expect(loadModelCatalogMock).toHaveBeenCalledTimes(2);
  });

  it("does not reload a catalog that has unmounted", async () => {
    loadModelCatalogMock
      .mockResolvedValueOnce(catalog("Sonnet", "~anthropic/claude-sonnet-latest"))
      .mockResolvedValueOnce(catalog("Sonnet", "~anthropic/claude-sonnet-latest"))
      .mockResolvedValueOnce(catalog("Luna", "openai/gpt-5.6-luna"));
    const unmounted = render(<Probe />);
    render(<Probe />);
    await waitFor(() => expect(screen.getAllByText("Sonnet")).toHaveLength(2));
    unmounted.unmount();

    await act(async () => {
      invalidateModelCatalog();
    });

    await screen.findByText("Luna");
    expect(loadModelCatalogMock).toHaveBeenCalledTimes(3);
  });

  it("still reloads on refresh after an invalidation", async () => {
    loadModelCatalogMock
      .mockResolvedValueOnce(catalog("Sonnet", "~anthropic/claude-sonnet-latest"))
      .mockResolvedValueOnce(catalog("Luna", "openai/gpt-5.6-luna"))
      .mockResolvedValueOnce(catalog("Private Qwen", "custom/018f0f45-6d2d-7b41-a741-a488b456805a"));
    const user = userEvent.setup();
    render(<Probe />);
    await screen.findByText("Sonnet");

    await act(async () => {
      invalidateModelCatalog();
    });
    await screen.findByText("Luna");
    await user.click(screen.getByRole("button", { name: "Refresh" }));

    await screen.findByText("Private Qwen");
    expect(loadModelCatalogMock).toHaveBeenCalledTimes(3);
  });
});

describe("useModelCatalog cache", () => {
  it("shows a reopened conversation's catalog at once and replaces it with the fresh answer", async () => {
    const fresh = deferredCatalog();
    loadModelCatalogMock
      .mockResolvedValueOnce(catalog("Sonnet", "~anthropic/claude-sonnet-latest"))
      .mockResolvedValueOnce(catalog("Private Qwen", "custom/018f0f45-6d2d-7b41-a741-a488b456805a"))
      .mockReturnValueOnce(fresh.promise);
    const { rerender } = render(<StateProbe conversationId="conversation-one" />);
    await screen.findByText("Sonnet");
    rerender(<StateProbe conversationId="conversation-two" />);
    await screen.findByText("Private Qwen");

    rerender(<StateProbe conversationId="conversation-one" />);

    expect(screen.getByText("Sonnet")).toBeInTheDocument();
    expect(screen.getByText("ready")).toBeInTheDocument();
    expect(loadModelCatalogMock).toHaveBeenCalledTimes(3);
    expect(loadModelCatalogMock).toHaveBeenNthCalledWith(3, "conversation-one");

    await act(async () => {
      fresh.resolve(catalog("Luna", "openai/gpt-5.6-luna"));
    });
    await screen.findByText("Luna");
    expect(screen.queryByText("Sonnet")).not.toBeInTheDocument();
  });

  it("drops the cached catalog on invalidation, so the next open of that conversation shows loading", async () => {
    loadModelCatalogMock.mockResolvedValueOnce(catalog("Sonnet", "~anthropic/claude-sonnet-latest"));
    const first = render(<StateProbe conversationId="conversation-one" />);
    await screen.findByText("Sonnet");
    first.unmount();

    invalidateModelCatalog();
    loadModelCatalogMock.mockReturnValueOnce(new Promise<ChatModelCatalogView>(() => {}));
    render(<StateProbe conversationId="conversation-one" />);

    expect(screen.getByText("loading")).toBeInTheDocument();
    expect(screen.queryByText("Sonnet")).not.toBeInTheDocument();
    expect(loadModelCatalogMock).toHaveBeenCalledTimes(2);
  });

  it("never shows a catalog cached for another account", async () => {
    setAuthSession(authenticated("account-one"));
    loadModelCatalogMock.mockResolvedValueOnce(catalog("Sonnet", "~anthropic/claude-sonnet-latest"));
    const { rerender } = render(<StateProbe conversationId="conversation-one" />);
    await screen.findByText("Sonnet");

    loadModelCatalogMock.mockReturnValueOnce(new Promise<ChatModelCatalogView>(() => {}));
    setAuthSession(authenticated("account-two"));
    rerender(<StateProbe conversationId="conversation-one" />);

    expect(screen.queryByText("Sonnet")).not.toBeInTheDocument();
    expect(screen.getByText("loading")).toBeInTheDocument();
    expect(loadModelCatalogMock).toHaveBeenNthCalledWith(2, "conversation-one");
  });

  it("keeps the cached catalog when the background revalidation fails", async () => {
    loadModelCatalogMock.mockResolvedValueOnce(catalog("Sonnet", "~anthropic/claude-sonnet-latest"));
    const first = render(<StateProbe conversationId="conversation-one" />);
    await screen.findByText("Sonnet");
    first.unmount();

    const revalidation = deferredCatalog();
    loadModelCatalogMock.mockReturnValueOnce(revalidation.promise);
    render(<StateProbe conversationId="conversation-one" />);
    expect(screen.getByText("Sonnet")).toBeInTheDocument();
    expect(screen.getByText("ready")).toBeInTheDocument();

    await act(async () => {
      revalidation.reject(new Error("offline"));
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    expect(screen.getByText("Sonnet")).toBeInTheDocument();
    expect(screen.getByText("ready")).toBeInTheDocument();
    expect(screen.queryByText("Could not load models.")).not.toBeInTheDocument();
  });

  it("keeps the 50 most recent catalogs and drops the oldest", async () => {
    for (let index = 0; index <= 50; index += 1) {
      loadModelCatalogMock.mockResolvedValueOnce(catalog(`Model ${index}`, `custom/model-${index}`));
      const { unmount } = render(<StateProbe conversationId={`conversation-${index}`} />);
      await screen.findByText(`Model ${index}`);
      unmount();
    }
    loadModelCatalogMock.mockReturnValue(new Promise<ChatModelCatalogView>(() => {}));

    const evicted = render(<StateProbe conversationId="conversation-0" />);
    expect(screen.getByText("loading")).toBeInTheDocument();
    evicted.unmount();

    render(<StateProbe conversationId="conversation-1" />);
    expect(screen.getByText("Model 1")).toBeInTheDocument();
  });

  it("keeps the catalog on screen while an invalidation reloads it, then shows the fresh answer", async () => {
    const reload = deferredCatalog();
    loadModelCatalogMock
      .mockResolvedValueOnce(catalog("Sonnet", "~anthropic/claude-sonnet-latest"))
      .mockReturnValueOnce(reload.promise);
    render(<StateProbe conversationId="conversation-one" />);
    await screen.findByText("Sonnet");

    await act(async () => {
      invalidateModelCatalog();
    });

    expect(screen.getByText("Sonnet")).toBeInTheDocument();
    expect(screen.getByText("ready")).toBeInTheDocument();
    expect(loadModelCatalogMock).toHaveBeenCalledTimes(2);

    await act(async () => {
      reload.resolve(catalog("Luna", "openai/gpt-5.6-luna"));
    });
    await screen.findByText("Luna");
    expect(screen.queryByText("Sonnet")).not.toBeInTheDocument();
  });

  it("keeps the shown catalog when the reload after an invalidation fails", async () => {
    const reload = deferredCatalog();
    loadModelCatalogMock
      .mockResolvedValueOnce(catalog("Sonnet", "~anthropic/claude-sonnet-latest"))
      .mockReturnValueOnce(reload.promise);
    render(<StateProbe conversationId="conversation-one" />);
    await screen.findByText("Sonnet");

    await act(async () => {
      invalidateModelCatalog();
    });
    await act(async () => {
      reload.reject(new Error("offline"));
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    expect(screen.getByText("Sonnet")).toBeInTheDocument();
    expect(screen.getByText("ready")).toBeInTheDocument();
    expect(screen.queryByText("Could not load models.")).not.toBeInTheDocument();
  });
});
