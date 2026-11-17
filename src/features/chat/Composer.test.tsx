import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useState } from "react";
import { Composer } from "./Composer";

afterEach(cleanup);

describe("Composer", () => {
  it("submits a trimmed prompt and clears the input", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();

    render(<Composer onSubmit={onSubmit} />);

    const input = screen.getByRole("textbox", { name: "Message xFalcon" });
    await user.type(input, "  Investigate this dataset  ");
    await user.click(screen.getByRole("button", { name: "Send message" }));

    expect(onSubmit).toHaveBeenCalledWith("Investigate this dataset");
    expect(input).toHaveValue("");
  });

  it("does not submit an empty prompt", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();

    render(<Composer onSubmit={onSubmit} />);

    await user.type(screen.getByRole("textbox", { name: "Message xFalcon" }), "   ");

    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("is unavailable when no message transport is connected", () => {
    render(<Composer />);

    expect(screen.getByRole("textbox", { name: "Message xFalcon" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
    expect(screen.getByText("Chat becomes available after the secure runtime connects.")).toBeInTheDocument();
    expect(screen.queryByText("Enter a message, then use the Send message button.")).not.toBeInTheDocument();
  });

  it("uses controlled draft when draft and onDraftChange are provided", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();

    function ControlledComposer() {
      const [draft, setDraft] = useState("seed");
      return <Composer draft={draft} onDraftChange={setDraft} onSubmit={onSubmit} />;
    }

    render(<ControlledComposer />);

    const input = screen.getByRole("textbox", { name: "Message xFalcon" });
    expect(input).toHaveValue("seed");

    await user.clear(input);
    await user.type(input, "controlled");
    expect(input).toHaveValue("controlled");

    await user.click(screen.getByRole("button", { name: "Send message" }));
    expect(onSubmit).toHaveBeenCalledWith("controlled");
    expect(input).toHaveValue("");
  });

  it("pastes plain multiline text over the current selection and restores the caret", () => {
    function ControlledComposer() {
      const [draft, setDraft] = useState("Review old value now");
      return <Composer draft={draft} onDraftChange={setDraft} onSubmit={vi.fn()} />;
    }

    render(<ControlledComposer />);
    const input = screen.getByRole("textbox", { name: "Message xFalcon" }) as HTMLTextAreaElement;
    input.setSelectionRange(7, 16);

    fireEvent.paste(input, {
      clipboardData: {
        getData: (type: string) => (type === "text/plain" ? "DIM_CUSTOMER\r\nDIM_PRODUCT" : ""),
      },
    });

    expect(input).toHaveValue("Review DIM_CUSTOMER\nDIM_PRODUCT now");
    expect(input.selectionStart).toBe("Review DIM_CUSTOMER\nDIM_PRODUCT".length);
    expect(input.selectionEnd).toBe(input.selectionStart);
  });

  it("includes HTML-only table and rich-text content in the submitted prompt", () => {
    const onSubmit = vi.fn();
    render(<Composer onSubmit={onSubmit} />);
    const input = screen.getByRole("textbox", { name: "Message xFalcon" });
    fireEvent.paste(input, {
      clipboardData: {
        getData: (type: string) =>
          type === "text/html"
            ? "<p>Compare <strong>sales &amp; margin</strong></p><table><tr><th>Region</th><th>Sales</th></tr><tr><td>East</td><td>120</td></tr></table><p>Explain<br>the change</p><script>untrusted()</script><style>p { color: red }</style>"
            : "",
      },
    });
    expect(input).toHaveValue("Compare sales & margin\nRegion\tSales\nEast\t120\nExplain\nthe change");
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    expect(onSubmit).toHaveBeenCalledWith("Compare sales & margin\nRegion\tSales\nEast\t120\nExplain\nthe change");
  });

  it("replaces the selection with HTML-only content and leaves image-only paste untouched", () => {
    render(<Composer onSubmit={vi.fn()} />);
    const input = screen.getByRole("textbox", { name: "Message xFalcon" }) as HTMLTextAreaElement;
    fireEvent.change(input, { target: { value: "Review old now" } });
    input.setSelectionRange(7, 10);
    fireEvent.paste(input, {
      clipboardData: { getData: (type: string) => (type === "text/html" ? "<p>new <b>table</b></p>" : "") },
    });
    expect(input).toHaveValue("Review new table now");
    expect(input.selectionStart).toBe("Review new table".length);
    expect(
      fireEvent.paste(input, {
        clipboardData: {
          getData: (type: string) => (type === "text/html" ? '<img src="https://example.test/image.png">' : ""),
        },
      }),
    ).toBe(true);
    expect(input).toHaveValue("Review new table now");
  });

  it("prefers the clipboard's plain-text representation over HTML", () => {
    render(<Composer onSubmit={vi.fn()} />);
    const input = screen.getByRole("textbox", { name: "Message xFalcon" });
    fireEvent.paste(input, {
      clipboardData: { getData: (type: string) => (type === "text/plain" ? "Exact\ttext" : "<p>Different</p>") },
    });
    expect(input).toHaveValue("Exact\ttext");
  });

  it.each(["files", "items"])(
    "attaches clipboard %s once without inserting accompanying text",
    async (representation) => {
      const onPasteFiles = vi.fn().mockResolvedValue(undefined);
      const file = new File(["a,b\n1,2"], "report.csv", { type: "text/csv" });
      render(<Composer onSubmit={vi.fn()} onPasteFiles={onPasteFiles} />);
      const input = screen.getByRole("textbox", { name: "Message xFalcon" });
      fireEvent.change(input, { target: { value: "Review this" } });
      await act(async () => {
        fireEvent.paste(input, {
          clipboardData: {
            files: representation === "files" ? [file] : [],
            items: [{ kind: "file", getAsFile: () => file }],
            getData: () => "Clipboard caption",
          },
        });
      });
      expect(onPasteFiles).toHaveBeenCalledExactlyOnceWith([file]);
      expect(input).toHaveValue("Review this");
    },
  );

  it.each([
    "report.doc",
    "report.DOC",
    "report.docx",
    "report.DOCX",
    "slides.ppt",
    "slides.PPT",
    "slides.pptx",
    "slides.PPTX",
  ])("passes Office clipboard file %s to the attachment flow", async (name) => {
    const onPasteFiles = vi.fn().mockResolvedValue(undefined);
    const file = new File(["office bytes"], name, { type: "application/octet-stream" });
    render(<Composer onSubmit={vi.fn()} onPasteFiles={onPasteFiles} />);
    const input = screen.getByRole("textbox", { name: "Message xFalcon" });

    await act(async () => {
      fireEvent.paste(input, {
        clipboardData: { files: [file], getData: () => "" },
      });
    });

    expect(onPasteFiles).toHaveBeenCalledExactlyOnceWith([file]);
    expect(input).toHaveValue("");
  });

  it("delegates native-only clipboard data without passing clipboard paths", async () => {
    const onPasteFiles = vi.fn().mockResolvedValue(undefined);
    render(<Composer onSubmit={vi.fn()} onPasteFiles={onPasteFiles} />);
    await act(async () => {
      fireEvent.paste(screen.getByRole("textbox", { name: "Message xFalcon" }), {
        clipboardData: { getData: (type: string) => (type === "text/uri-list" ? "file:///private/report.xlsx" : "") },
      });
    });
    expect(onPasteFiles).toHaveBeenCalledExactlyOnceWith([]);
  });

  it("prefers native file references over a file manager's accompanying filename text", async () => {
    const onPasteFiles = vi.fn().mockResolvedValue(undefined);
    render(<Composer onSubmit={vi.fn()} onPasteFiles={onPasteFiles} />);
    const input = screen.getByRole("textbox", { name: "Message xFalcon" });
    await act(async () => {
      fireEvent.paste(input, {
        clipboardData: {
          types: ["Files", "text/plain"],
          getData: (type: string) => (type === "text/plain" ? "report.xlsx" : ""),
        },
      });
    });
    expect(onPasteFiles).toHaveBeenCalledExactlyOnceWith([]);
    expect(input).toHaveValue("");
  });

  it.each([{ streaming: true }, { submitting: true }, { disabled: true }])(
    "blocks file paste when unavailable: %j",
    (state) => {
      const onPasteFiles = vi.fn();
      render(<Composer {...state} onSubmit={vi.fn()} onPasteFiles={onPasteFiles} />);
      fireEvent.paste(screen.getByRole("textbox", { name: "Message xFalcon" }), {
        clipboardData: { files: [new File(["text"], "note.txt")], getData: () => "" },
      });
      expect(onPasteFiles).not.toHaveBeenCalled();
    },
  );

  it("blocks send and repeated file paste while clipboard admission is pending", async () => {
    let finish!: () => void;
    const onPasteFiles = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    );
    const onSubmit = vi.fn();
    render(<Composer onSubmit={onSubmit} onAttach={vi.fn()} onPasteFiles={onPasteFiles} />);
    const input = screen.getByRole("textbox", { name: "Message xFalcon" });
    fireEvent.change(input, { target: { value: "Review" } });
    const clipboardData = { files: [new File(["text"], "note.txt")], getData: () => "" };
    fireEvent.paste(input, { clipboardData });
    fireEvent.paste(input, { clipboardData });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onSubmit).not.toHaveBeenCalled();
    expect(input).toHaveValue("Review");
    expect(onPasteFiles).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
    fireEvent.paste(input, { clipboardData: { getData: () => " typing" } });
    expect(input).toHaveValue("Review typing");
    await act(async () => finish());
    expect(screen.getByRole("button", { name: "Send message" })).toBeEnabled();
  });

  it("sends on Enter", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();

    render(<Composer onSubmit={onSubmit} />);

    const input = screen.getByRole("textbox", { name: "Message xFalcon" });
    await user.type(input, "Ship it{Enter}");

    expect(onSubmit).toHaveBeenCalledWith("Ship it");
    expect(input).toHaveValue("");
  });

  it("inserts a newline on Shift+Enter instead of sending", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();

    render(<Composer onSubmit={onSubmit} />);

    const input = screen.getByRole("textbox", { name: "Message xFalcon" });
    await user.type(input, "line one{Shift>}{Enter}{/Shift}line two");

    expect(onSubmit).not.toHaveBeenCalled();
    expect(input).toHaveValue("line one\nline two");
  });

  it("does not send an Enter that commits an IME composition", () => {
    const onSubmit = vi.fn();

    render(<Composer onSubmit={onSubmit} />);

    const input = screen.getByRole("textbox", { name: "Message xFalcon" });
    fireEvent.change(input, { target: { value: "こんにちは" } });
    fireEvent.keyDown(input, { key: "Enter", isComposing: true });

    expect(onSubmit).not.toHaveBeenCalled();
    expect(input).toHaveValue("こんにちは");
  });

  it("renders the minimal card: attach affordance and a send disabled while empty", () => {
    render(<Composer onSubmit={vi.fn()} />);

    expect(screen.getByRole("textbox", { name: "Message xFalcon" })).toHaveAttribute(
      "placeholder",
      "\u00a0\u00a0Ask about data",
    );
    expect(screen.getByRole("button", { name: /attach files/i })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
  });

  it("shows the chat model as read-only status with help on hover", async () => {
    const user = userEvent.setup();
    render(<Composer modelName="Grok" onSubmit={vi.fn()} />);

    expect(screen.getByLabelText("Model: Grok")).toHaveTextContent("Grok");
    expect(screen.queryByRole("button", { name: /grok/i })).not.toBeInTheDocument();
    await user.hover(screen.getByLabelText("Model: Grok"));
    expect(await screen.findByRole("tooltip")).toHaveTextContent(
      "Change models in Settings. A chat keeps the model it starts with.",
    );
  });

  it("says what the model runs on beside its name when a note is given", () => {
    render(<Composer modelName="GPT Luna" modelNote="Using ChatGPT plan" onSubmit={vi.fn()} />);

    expect(screen.getByLabelText("Model: GPT Luna")).toHaveTextContent("GPT Luna");
    expect(screen.getByText("Using ChatGPT plan")).toBeInTheDocument();
  });

  it("shows no plan note for a model without one", () => {
    render(<Composer modelName="Grok" onSubmit={vi.fn()} />);

    expect(screen.getByLabelText("Model: Grok")).toBeInTheDocument();
    expect(screen.queryByText(/Using ChatGPT plan/)).not.toBeInTheDocument();
  });

  it("enables attach when the composer can send", () => {
    render(<Composer onAttach={vi.fn()} onSubmit={vi.fn()} />);

    expect(screen.getByRole("button", { name: "Attach files" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
  });

  it("keeps the input scrollbar-free until the draft outgrows the cap", () => {
    // `scrollHeight` is the only thing the autosize reads, and jsdom reports
    // 0 for it - stub it to stand in for a one-line draft and a long one.
    const scrollHeight = vi.spyOn(Element.prototype, "scrollHeight", "get");

    scrollHeight.mockReturnValue(32);
    const { unmount } = render(<Composer draft="one line" onDraftChange={vi.fn()} onSubmit={vi.fn()} />);
    const short = screen.getByRole("textbox", { name: "Message xFalcon" });
    expect(short.style.overflowY).toBe("hidden");
    expect(short.style.height).toBe("32px");
    unmount();

    scrollHeight.mockReturnValue(480);
    render(<Composer draft={"line\n".repeat(40)} onDraftChange={vi.fn()} onSubmit={vi.fn()} />);
    const grown = screen.getByRole("textbox", { name: "Message xFalcon" });
    expect(grown.style.overflowY).toBe("auto");
    expect(grown.style.height).toBe("330px");

    scrollHeight.mockRestore();
  });

  it("shows the active project as a compact, quiet label above the input", () => {
    render(<Composer activeProjectName="Research" onSubmit={vi.fn()} />);

    expect(screen.getByText("Research")).toBeInTheDocument();
  });

  it("lets the user remove project context from the composer", async () => {
    const user = userEvent.setup();
    const onClearProject = vi.fn();
    render(<Composer activeProjectName="Research" onClearProject={onClearProject} onSubmit={vi.fn()} />);

    await user.click(screen.getByRole("button", { name: "Remove Research project context" }));

    expect(onClearProject).toHaveBeenCalledOnce();
  });

  it("renders no project label when no project is active", () => {
    render(<Composer onSubmit={vi.fn()} />);

    expect(screen.queryByText("Research")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /remove .* project context/i })).not.toBeInTheDocument();
  });

  describe("autosize (spec: chat-composer)", () => {
    const mockScrollHeight = (value: number) => {
      Object.defineProperty(HTMLTextAreaElement.prototype, "scrollHeight", {
        configurable: true,
        get: () => value,
      });
    };
    afterEach(() => {
      // Restore the jsdom default (undefined getter on the instance chain).
      delete (HTMLTextAreaElement.prototype as unknown as Record<string, unknown>)["scrollHeight"];
    });

    it("grows with the draft and caps at 330px", () => {
      mockScrollHeight(140);
      render(<Composer onSubmit={vi.fn()} />);
      const input = screen.getByRole("textbox", { name: "Message xFalcon" });

      fireEvent.change(input, { target: { value: "line\n".repeat(6) } });
      expect((input as HTMLTextAreaElement).style.height).toBe("140px");

      mockScrollHeight(900);
      fireEvent.change(input, { target: { value: "line\n".repeat(60) } });
      expect((input as HTMLTextAreaElement).style.height).toBe("330px");
    });

    it("resets to its natural height after send", async () => {
      mockScrollHeight(140);
      const user = userEvent.setup();
      render(<Composer onSubmit={vi.fn()} />);
      const input = screen.getByRole("textbox", { name: "Message xFalcon" });

      fireEvent.change(input, { target: { value: "a long draft" } });
      expect((input as HTMLTextAreaElement).style.height).toBe("140px");

      mockScrollHeight(24);
      await user.click(screen.getByRole("button", { name: "Send message" }));
      expect((input as HTMLTextAreaElement).style.height).toBe("24px");
    });

    it("does not resize mid-IME-composition, then settles on compositionend", () => {
      mockScrollHeight(140);
      render(<Composer onSubmit={vi.fn()} />);
      const input = screen.getByRole("textbox", { name: "Message xFalcon" });

      fireEvent.change(input, { target: { value: "seed" } });
      expect((input as HTMLTextAreaElement).style.height).toBe("140px");

      mockScrollHeight(220);
      fireEvent.compositionStart(input);
      fireEvent.change(input, { target: { value: "seed かな" } });
      expect((input as HTMLTextAreaElement).style.height).toBe("140px");

      fireEvent.compositionEnd(input);
      expect((input as HTMLTextAreaElement).style.height).toBe("220px");
    });
  });

  describe("arrival", () => {
    it("puts the caret in the composer, so typing costs no click", () => {
      render(<Composer onSubmit={vi.fn()} />);
      expect(screen.getByRole("textbox", { name: "Message xFalcon" })).toHaveFocus();
    });

    it("does not take focus while the runtime cannot accept a message", () => {
      render(<Composer disabled onSubmit={vi.fn()} />);
      expect(screen.getByRole("textbox", { name: "Message xFalcon" })).not.toHaveFocus();
    });

    // Focus is taken once. A later re-render must not be able to pull the
    // caret back out of a dialog, the transcript, or wherever the reader put
    // it.
    it("does not re-take focus on a later render", () => {
      const { rerender } = render(<Composer onSubmit={vi.fn()} />);
      const input = screen.getByRole("textbox", { name: "Message xFalcon" });
      input.blur();

      rerender(<Composer onSubmit={vi.fn()} streaming />);
      expect(input).not.toHaveFocus();
    });

    it("returns focus to the composer once the response finishes", async () => {
      const { rerender } = render(<Composer onSubmit={vi.fn()} streaming />);
      const input = screen.getByRole("textbox", { name: "Message xFalcon" });
      const elsewhere = document.createElement("button");
      document.body.appendChild(elsewhere);
      elsewhere.focus();

      rerender(<Composer onSubmit={vi.fn()} />);
      await Promise.resolve();

      expect(input).toHaveFocus();
      elsewhere.remove();
    });

    it("does not steal completion focus from another text field", async () => {
      const { rerender } = render(<Composer onSubmit={vi.fn()} streaming />);
      const otherInput = document.createElement("input");
      document.body.appendChild(otherInput);
      otherInput.focus();

      rerender(<Composer onSubmit={vi.fn()} />);
      await Promise.resolve();

      expect(otherInput).toHaveFocus();
      otherInput.remove();
    });
  });

  describe("palette & slash commands", () => {
    it("opens slash command palette on / and executes command via onSlashCommand", async () => {
      const user = userEvent.setup();
      const onSlashCommand = vi.fn();
      const onSubmit = vi.fn();

      render(<Composer onSlashCommand={onSlashCommand} onSubmit={onSubmit} />);

      const input = screen.getByRole("textbox", { name: "Message xFalcon" });
      await user.type(input, "/");

      expect(screen.getByRole("listbox", { name: "Slash commands" })).toBeInTheDocument();
      expect(screen.getByText("/clear")).toBeInTheDocument();
      expect(screen.getByText("/help")).toBeInTheDocument();

      // Click on /clear
      await user.click(screen.getByText("/clear"));

      expect(onSlashCommand).toHaveBeenCalledWith("clear");
      expect(onSubmit).not.toHaveBeenCalled();
      expect(input).toHaveValue("");
    });

    it("triggers onSlashCommand when /clear or /help is submitted directly on Enter", async () => {
      const user = userEvent.setup();
      const onSlashCommand = vi.fn();
      const onSubmit = vi.fn();

      render(<Composer onSlashCommand={onSlashCommand} onSubmit={onSubmit} />);

      const input = screen.getByRole("textbox", { name: "Message xFalcon" });
      await user.type(input, "/help{Enter}");

      expect(onSlashCommand).toHaveBeenCalledWith("help");
      expect(onSubmit).not.toHaveBeenCalled();
      expect(input).toHaveValue("");
    });

    it("opens file & mcp mentions on @ and inserts selection", async () => {
      const user = userEvent.setup();
      const onSubmit = vi.fn();

      render(
        <Composer mcpServers={["github-tools"]} projectFiles={["sales_q3.csv", "report.docx"]} onSubmit={onSubmit} />,
      );

      const input = screen.getByRole("textbox", { name: "Message xFalcon" });
      await user.type(input, "@");

      expect(screen.getByRole("listbox")).toBeInTheDocument();
      expect(screen.getByText("@sales_q3.csv")).toBeInTheDocument();
      expect(screen.getByText("@mcp:github-tools")).toBeInTheDocument();

      await user.click(screen.getByText("@sales_q3.csv"));
      expect(input).toHaveValue("@sales_q3.csv ");
    });

    it("keeps the caret after an inserted mention, so following text stays outside it", async () => {
      const user = userEvent.setup();
      render(<Composer onSubmit={vi.fn()} projectFiles={["sales_q3.csv"]} />);

      const input = screen.getByRole("textbox", { name: "Message xFalcon" });
      await user.type(input, "Review @sal");
      await user.click(screen.getByText("@sales_q3.csv"));
      await user.type(input, "next");

      expect(input).toHaveValue("Review @sales_q3.csv next");
      expect((input as HTMLTextAreaElement).selectionStart).toBe("Review @sales_q3.csv next".length);
    });

    it("keeps an empty suggestion surface dismissible with Escape", async () => {
      const user = userEvent.setup();
      render(<Composer onSubmit={vi.fn()} projectFiles={["sales_q3.csv"]} />);

      const input = screen.getByRole("textbox", { name: "Message xFalcon" });
      await user.type(input, "@no-match");

      expect(screen.getByRole("listbox", { name: "Mention suggestions" })).toHaveTextContent(
        "No matching files or resources found",
      );
      expect(input).toHaveAttribute("aria-expanded", "true");
      expect(input).toHaveAttribute("aria-controls", "composer-palette-list");

      await user.keyboard("{Escape}");

      expect(screen.queryByRole("listbox", { name: "Mention suggestions" })).not.toBeInTheDocument();
      expect(input).toHaveAttribute("aria-expanded", "false");
      expect(input).not.toHaveAttribute("aria-controls");
    });

    it("resets the active suggestion when the palette changes from mentions to commands", async () => {
      const user = userEvent.setup();
      const onSlashCommand = vi.fn();
      render(
        <Composer
          onSlashCommand={onSlashCommand}
          onSubmit={vi.fn()}
          projectFiles={["one.csv", "two.csv", "three.csv", "four.csv", "five.csv", "six.csv"]}
        />,
      );

      const input = screen.getByRole("textbox", { name: "Message xFalcon" });
      await user.type(input, "@");
      await user.keyboard("{ArrowDown}{ArrowDown}{ArrowDown}{ArrowDown}{ArrowDown}");
      await user.clear(input);
      await user.type(input, "/");
      await user.keyboard("{Enter}");

      expect(onSlashCommand).toHaveBeenCalledWith("model");
    });

    it("does not trigger mention palette on email addresses", async () => {
      const user = userEvent.setup();
      render(<Composer onSubmit={vi.fn()} />);

      const input = screen.getByRole("textbox", { name: "Message xFalcon" });
      await user.type(input, "contact user@example.com");

      expect(screen.queryByRole("listbox")).toBeNull();
    });

    it("lists imported skills on /skill and inserts @skill on pick", async () => {
      const user = userEvent.setup();
      const onSlashCommand = vi.fn();
      render(
        <Composer
          onSlashCommand={onSlashCommand}
          onSubmit={vi.fn()}
          skills={[{ name: "financial-analysis", description: "Read the books" }]}
        />,
      );

      const input = screen.getByRole("textbox", { name: "Message xFalcon" });
      await user.type(input, "/skill{Enter}");

      expect(onSlashCommand).not.toHaveBeenCalled();
      expect(screen.getByRole("listbox")).toBeInTheDocument();
      expect(screen.getByText("@skill:financial-analysis")).toBeInTheDocument();

      await user.click(screen.getByText("@skill:financial-analysis"));
      expect(input).toHaveValue("@skill:financial-analysis ");
    });

    it("filters mentions to skills when the query starts with skill", async () => {
      const user = userEvent.setup();
      render(
        <Composer
          mcpServers={["github-tools"]}
          onSubmit={vi.fn()}
          projectFiles={["sales.csv"]}
          skills={[{ name: "financial-analysis", description: "Read the books" }]}
        />,
      );

      const input = screen.getByRole("textbox", { name: "Message xFalcon" });
      await user.type(input, "@skill");

      expect(screen.getByText("@skill:financial-analysis")).toBeInTheDocument();
      expect(screen.queryByText("@sales.csv")).toBeNull();
      expect(screen.queryByText("@mcp:github-tools")).toBeNull();
    });
  });

  describe("attachments (spec: composer-uploads)", () => {
    const ready = {
      version: 1 as const,
      id: "upload-1",
      purpose: "chat_attachment" as const,
      state: "ready" as const,
      bytes_sent: 12,
      size: 12,
      display_name: "chart.png",
      safe_error: null,
      resource_id: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
      conversation_id: "conv-1",
    };

    it("submits a ready file with an empty prompt", async () => {
      const user = userEvent.setup();
      const onSubmit = vi.fn();
      render(<Composer attachments={[ready]} onAttach={vi.fn()} onSubmit={onSubmit} />);

      expect(screen.getByText("chart.png")).toBeInTheDocument();
      await user.click(screen.getByRole("button", { name: "Send message" }));

      expect(onSubmit).toHaveBeenCalledWith("", ["aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"]);
    });

    it("blocks send while an attachment is still transferring", () => {
      render(
        <Composer
          attachments={[{ ...ready, state: "uploading", bytes_sent: 4, resource_id: undefined }]}
          onAttach={vi.fn()}
          onSubmit={vi.fn()}
        />,
      );

      expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
    });

    it("cancels and retries from the chip", async () => {
      const user = userEvent.setup();
      const onCancelUpload = vi.fn();
      const onRetryUpload = vi.fn();
      render(
        <Composer
          attachments={[{ ...ready, state: "failed", safe_error: "interrupted", resource_id: undefined }]}
          onAttach={vi.fn()}
          onCancelUpload={onCancelUpload}
          onRetryUpload={onRetryUpload}
          onSubmit={vi.fn()}
        />,
      );

      await user.click(screen.getByRole("button", { name: "Retry chart.png" }));
      await user.click(screen.getByRole("button", { name: "Remove chart.png" }));

      expect(onRetryUpload).toHaveBeenCalledWith("upload-1");
      expect(onCancelUpload).toHaveBeenCalledWith("upload-1");
    });

    it("shows the safe error returned for a failed upload", () => {
      render(
        <Composer
          attachments={[
            { ...ready, state: "failed", safe_error: "The workspace refused this file.", resource_id: undefined },
          ]}
          onAttach={vi.fn()}
          onSubmit={vi.fn()}
        />,
      );

      expect(screen.getByRole("status")).toHaveTextContent("Upload failed: The workspace refused this file.");
    });
  });
});
