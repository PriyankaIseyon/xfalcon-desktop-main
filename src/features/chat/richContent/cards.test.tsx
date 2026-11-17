import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { previewDocumentRegister } from "../../../lib/native";
import { writeClipboardText } from "../clipboard";
import {
  ArtifactFrame,
  CodeArtifact,
  HtmlArtifact,
  MarkdownArtifact,
  SvgArtifact,
  TABLE_ROW_CAP,
  TableArtifact,
} from "./cards";
import { ARTIFACT_ACTION_CLASS } from "./artifactAction";

vi.mock("../../../lib/native", () => ({
  previewDocumentRegister: vi.fn(async () => "cafe".repeat(16)),
  previewDocumentSrc: (id: string) => `http://xf-preview.localhost/${id}`,
}));

vi.mock("../clipboard", () => ({
  writeClipboardText: vi.fn(async () => undefined),
}));

afterEach(cleanup);
beforeEach(() => {
  vi.mocked(previewDocumentRegister).mockClear();
  vi.mocked(previewDocumentRegister).mockImplementation(async () => "cafe".repeat(16));
  vi.mocked(writeClipboardText).mockClear();
});

const DOC = `<!DOCTYPE html>\n<html><head><title>Report</title></head><body><script>window.parent.__TAURI_INTERNALS__</script>${"x".repeat(40)}</body></html>`;

const table = (rows: string[][], headers = ["n"]) => ({ headers, rows });

describe("ArtifactFrame — one chrome for every artifact (spec: chat-rich-content)", () => {
  it("gives four artifact types one header each, none of them type-specific", async () => {
    render(
      <>
        <HtmlArtifact html={DOC} />
        <TableArtifact source="n\n1" table={table([["1"]])} title="Data" />
        <MarkdownArtifact source="# Hi" sourceView="# Hi">
          <p>Hi</p>
        </MarkdownArtifact>
        <CodeArtifact code="SELECT 1;" language="sql">
          SELECT 1;
        </CodeArtifact>
      </>,
    );
    await screen.findByTitle("Report");

    const headers = document.querySelectorAll(".rich-card__header");
    expect(headers).toHaveLength(4);
    // jsdom does not lay out, so the equal-height contract is asserted where
    // it lives: the band's height is a CSS constant on this one class, and no
    // type may override it from the tree.
    for (const header of headers) {
      expect(header.getAttribute("style")).toBeNull();
      expect(header.querySelectorAll(".rich-card__title")).toHaveLength(1);
    }
  });

  it("renders a hostile title as inert clamped text and keeps the action row", () => {
    const hostile = `${"A".repeat(4000)}\n<img src=x onerror=alert(1)>`;
    const { container } = render(
      <ArtifactFrame source="s" title={hostile}>
        <p>body</p>
      </ArtifactFrame>,
    );
    const title = container.querySelector(".rich-card__title");
    expect(title?.textContent).toBe(hostile);
    expect(container.querySelector(".rich-card__title img")).toBeNull();
    expect(container.querySelector(".rich-card__actions")).not.toBeNull();
  });

  it("omits the description slot rather than filling it", () => {
    const { container } = render(
      <ArtifactFrame title="Code">
        <p>body</p>
      </ArtifactFrame>,
    );
    expect(container.querySelector(".rich-card__meta")).toBeNull();
  });
});

describe("The action row offers only actions that can act", () => {
  it("gives the code artifact copy and no source toggle — its body IS the source", () => {
    render(
      <CodeArtifact code="SELECT 1;" language="sql">
        SELECT 1;
      </CodeArtifact>,
    );
    expect(screen.getByRole("button", { name: "Copy sql to clipboard" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /source/ })).toBeNull();
  });

  it("offers nothing at all while the artifact is still arriving", () => {
    render(<HtmlArtifact building html={"<html><head><title>Report</title></head><body>"} />);
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("dresses every action, including the one supplied from outside, identically", () => {
    // The drift guard. "Save to project" is contributed by the reports feature
    // and used to carry a byte-identical copy of the frame's own class literal,
    // with a comment promising to keep the two in step. One exported name now,
    // so a change to the control's resting affordance reaches all three.
    render(
      <ArtifactFrame
        saveAction={
          <button className={ARTIFACT_ACTION_CLASS} type="button">
            Save to project
          </button>
        }
        source="s"
        sourceView="s"
        title="Report"
      >
        <p>body</p>
      </ArtifactFrame>,
    );
    const names = ["Show source of Report", "Copy Report to clipboard", "Save to project"];
    for (const name of names) {
      expect(screen.getByRole("button", { name })).toHaveClass(ARTIFACT_ACTION_CLASS);
    }
  });

  it("names each action for its own artifact so several frames are told apart", () => {
    render(
      <>
        <CodeArtifact code="SELECT 1;" language="sql">
          SELECT 1;
        </CodeArtifact>
        <TableArtifact source="n" sourceView="n" table={table([["1"]])} title="Data" />
        <MarkdownArtifact source="# Hi" sourceView="# Hi">
          <p>Hi</p>
        </MarkdownArtifact>
      </>,
    );
    const names = screen.getAllByRole("button", { name: /^Copy / }).map((button) => button.getAttribute("aria-label"));
    expect(names).toEqual(["Copy sql to clipboard", "Copy Data to clipboard", "Copy Markdown to clipboard"]);
  });

  it("copies exactly the string the source view shows", async () => {
    const user = userEvent.setup();
    render(<HtmlArtifact html={DOC} />);
    await screen.findByTitle("Report");

    await user.click(screen.getByRole("button", { name: "Copy Report to clipboard" }));
    expect(vi.mocked(writeClipboardText)).toHaveBeenCalledWith(DOC);

    await user.click(screen.getByRole("button", { name: "Show source of Report" }));
    expect(document.querySelector(".rich-card__code")?.textContent).toBe(DOC);
  });

  it("hides the preview rather than unmounting it, so a toggle costs no re-registration", async () => {
    const user = userEvent.setup();
    render(<HtmlArtifact html={DOC} />);
    const frame = await screen.findByTitle("Report");
    expect(vi.mocked(previewDocumentRegister)).toHaveBeenCalledTimes(1);

    await user.click(screen.getByRole("button", { name: "Show source of Report" }));
    await user.click(screen.getByRole("button", { name: "Hide source of Report" }));

    expect(screen.getByTitle("Report")).toBe(frame);
    expect(vi.mocked(previewDocumentRegister)).toHaveBeenCalledTimes(1);
  });

  it("offers no toggle once a preview has degraded — there is no body to return to", async () => {
    vi.mocked(previewDocumentRegister).mockRejectedValueOnce(new Error("store unavailable"));
    const { container } = render(<HtmlArtifact html={DOC} />);

    expect(await screen.findByText(/Preview unavailable — showing source/)).toBeInTheDocument();
    expect(container.querySelector("iframe")).toBeNull();
    expect(screen.queryByRole("button", { name: /source of/ })).toBeNull();
    expect(container.querySelector(".rich-card__code")?.textContent).toBe(DOC);
  });

  it("offers no toggle for a document over the preview cap, and never registers it", async () => {
    const huge = `<!DOCTYPE html><html><head><title>Huge</title></head><body>${"x".repeat(2 * 1024 * 1024)}</body></html>`;
    render(<HtmlArtifact html={huge} />);

    expect(await screen.findByText(/Document too large to preview — showing source/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /source of/ })).toBeNull();
    expect(vi.mocked(previewDocumentRegister)).not.toHaveBeenCalled();
  });

  it("offers the supported preview actions and no invented ones once registration completes", async () => {
    render(<HtmlArtifact html={DOC} />);
    await screen.findByTitle("Report");

    expect(screen.getByRole("button", { name: "Copy Report to clipboard" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open Report in the side panel" })).toBeInTheDocument();
    for (const forbidden of [/download/i, /share/i, /refresh/i]) {
      expect(screen.queryByRole("button", { name: forbidden })).toBeNull();
    }
  });
});

describe("The preview bodies keep one path and the isolation contract", () => {
  it("frames an html document on the preview origin — allow-scripts ONLY, never a local scheme", async () => {
    render(<HtmlArtifact html={DOC} />);
    const frame = await screen.findByTitle("Report");

    expect(frame.tagName).toBe("IFRAME");
    // Scripts run, but the document must not reach the app — no
    // allow-same-origin, and no local-scheme URL (data:/blob:/srcdoc inherit
    // the app CSP in WebView2, which refuses the document's own styling).
    expect(frame.getAttribute("sandbox")).toBe("allow-scripts");
    expect(frame.getAttribute("src")).toBe(`http://xf-preview.localhost/${"cafe".repeat(16)}`);
    expect(frame.getAttribute("srcdoc")).toBeNull();
    expect(vi.mocked(previewDocumentRegister)).toHaveBeenCalledWith("html", DOC);
  });

  it("frames an svg document on the same one path", async () => {
    const svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8"><rect width="8" height="8"/></svg>';
    render(<SvgArtifact svg={svg} />);
    const frame = await screen.findByTitle("SVG image");

    expect(frame.getAttribute("sandbox")).toBe("allow-scripts");
    expect(frame.getAttribute("src")).toMatch(/^http:\/\/xf-preview\.localhost\//);
    expect(vi.mocked(previewDocumentRegister)).toHaveBeenCalledWith("svg", svg);
  });

  it("never exposes an older registration after the artifact content changes", async () => {
    let finishFirst: (id: string) => void = () => {};
    vi.mocked(previewDocumentRegister)
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            finishFirst = resolve;
          }),
      )
      .mockResolvedValueOnce("new-document");
    const first = DOC;
    const second = DOC.replace("Report", "Updated report");
    const view = render(<HtmlArtifact html={first} />);

    view.rerender(<HtmlArtifact html={second} />);
    const frame = await screen.findByTitle("Updated report");
    expect(frame).toHaveAttribute("src", "http://xf-preview.localhost/new-document");

    finishFirst("old-document");
    await Promise.resolve();
    expect(screen.getByTitle("Updated report")).toHaveAttribute("src", "http://xf-preview.localhost/new-document");
  });
});

describe("The building state is the settled state's own component", () => {
  it("names the artifact the same before and after the body settles", async () => {
    const partial = "<!DOCTYPE html>\n<html><head><title>Report</title></head><body>";
    const { rerender } = render(<HtmlArtifact building html={partial} />);
    expect(screen.getByText("Report")).toBeInTheDocument();
    expect(screen.getByText("Building…")).toBeInTheDocument();

    rerender(<HtmlArtifact html={DOC} />);
    await screen.findByTitle("Report");
    // Only the body and the description changed.
    expect(screen.getByText("Report")).toBeInTheDocument();
    expect(screen.queryByText("Building…")).toBeNull();
  });

  it("reserves the settled frame height while building, and never registers a partial document", () => {
    const { container } = render(<HtmlArtifact building html={"<html><head><title>Report</title></head>"} />);
    expect(container.querySelector(".rich-card__frame-ghost")).not.toBeNull();
    expect(vi.mocked(previewDocumentRegister)).not.toHaveBeenCalled();
  });

  it("is a status while building and not one after", async () => {
    const { rerender } = render(<HtmlArtifact building html={"<html><head><title>Report</title></head>"} />);
    expect(screen.getByRole("status")).toBeInTheDocument();

    rerender(<HtmlArtifact html={DOC} />);
    await screen.findByTitle("Report");
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("invents no numbers for a table that has not parsed", () => {
    render(<TableArtifact building title="Data" />);
    expect(screen.getByText("Building…")).toBeInTheDocument();
    expect(screen.queryByText(/row/)).toBeNull();
    expect(screen.queryByText(/column/)).toBeNull();
  });
});

describe("TableArtifact", () => {
  it("states the artifact's true shape, not what the body renders", () => {
    const rows = Array.from({ length: 540 }, (_, index) => [String(index), "x"]);
    const { container } = render(
      <TableArtifact source="s" sourceView="s" table={table(rows, ["n", "label"])} title="Query results" />,
    );
    // The three numbers agree about one table: 540 total, 100 shown, 440 left.
    expect(screen.getByText("540 rows · 2 columns")).toBeInTheDocument();
    expect(container.querySelectorAll("tbody tr")).toHaveLength(TABLE_ROW_CAP);
    expect(screen.getByText(/440 more rows/)).toBeInTheDocument();
  });

  it("agrees in the singular", () => {
    render(<TableArtifact source="s" table={table([["1"]])} title="Data" />);
    expect(screen.getByText("1 row · 1 column")).toBeInTheDocument();
  });

  it("renders hostile cell content as inert text", () => {
    const { container } = render(
      <TableArtifact
        source="irrelevant"
        table={table([["<script>alert(1)</script>"], ["<img onerror=x src=y>"]], ["payload"])}
        title="Data"
      />,
    );
    expect(screen.getByText("<script>alert(1)</script>")).toBeInTheDocument();
    expect(container.querySelector("table script, table img")).toBeNull();
  });

  /** Which columns the table right-aligned, by header text. */
  const alignedRight = (container: HTMLElement) =>
    [...container.querySelectorAll("thead th")]
      .filter((cell) => cell.classList.contains("is-numeric"))
      .map((cell) => cell.textContent);

  describe("chat table columns stay on the leading edge", () => {
    it("does not right-align measure columns in the inline card", () => {
      const { container } = render(
        <TableArtifact
          source="s"
          table={table(
            [
              ["EMEA", "1,200.50", "12%", "$4,182"],
              ["APAC", "980.00", "8%", "$1,204"],
            ],
            ["region", "net", "share", "revenue"],
          )}
          title="Query results"
        />,
      );
      expect(alignedRight(container)).toEqual([]);
    });

    it("does not right-align numeric id columns", () => {
      const { container } = render(
        <TableArtifact
          source="s"
          table={table(
            [
              ["1", "Widget", "9.99"],
              ["2", "Gadget", "19.99"],
            ],
            ["id", "name", "price"],
          )}
          title="Query results"
        />,
      );
      expect(alignedRight(container)).toEqual([]);
    });

    it("does not mark a middle label column as the slack column when the last column is a measure", () => {
      const { container } = render(
        <TableArtifact
          source="s"
          table={table(
            [
              ["1", "Acme, Inc", "East", "1200"],
              ["2", "Beta LLC", "West", "950"],
            ],
            ["id", "name", "region", "amount"],
          )}
          title="Data"
        />,
      );
      const headers = [...container.querySelectorAll("thead th")];
      expect(headers.map((cell) => cell.className)).toEqual(["", "", "", ""]);
      const cells = [...container.querySelectorAll("tbody td")];
      expect(cells.every((cell) => cell.className === "")).toBe(true);
    });
  });

  describe("measure classification for canvas and charts", () => {
    it("offers Chart in panel when a table has labels and plottable measures", () => {
      render(
        <TableArtifact
          source="s"
          table={table(
            [
              ["1", "Alpha", "EMEA", "1200"],
              ["2", "Beta", "APAC", "950"],
            ],
            ["id", "name", "region", "amount"],
          )}
          title="Data"
        />,
      );
      expect(screen.getByRole("button", { name: "Chart Data in the side panel" })).toBeInTheDocument();
    });

    it("offers Chart in panel for json-shaped tables with one label column", () => {
      render(
        <TableArtifact
          source="s"
          table={table(
            [
              ["1", "Widget", "9.99"],
              ["2", "Gadget", "19.99"],
            ],
            ["id", "name", "price"],
          )}
          title="Query results"
        />,
      );
      expect(screen.getByRole("button", { name: "Chart Query results in the side panel" })).toBeInTheDocument();
    });

    it("withholds Chart in panel when no plottable measure exists", () => {
      render(
        <TableArtifact
          source="s"
          table={table(
            [
              ["1", "Alice", "vip"],
              ["2", "Bob", "regular"],
            ],
            ["id", "name", "note"],
          )}
          title="Data"
        />,
      );
      expect(screen.queryByRole("button", { name: /Chart Data in the side panel/i })).toBeNull();
    });
  });

  it("can be read back, not only copied", async () => {
    const user = userEvent.setup();
    render(<TableArtifact source="n\n1" sourceView="n\n1" table={table([["1"]])} title="Data" />);

    expect(screen.getByRole("table")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Show source of Data" }));
    expect(document.querySelector(".rich-card__code")?.textContent).toBe("n\\n1");
  });
});
