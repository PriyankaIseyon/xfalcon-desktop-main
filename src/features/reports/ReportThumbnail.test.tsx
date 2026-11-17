import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ReportThumbnail } from "./ReportThumbnail";
import { clearStoredThumbnails, storedThumbnailUrl } from "./storedThumbnails";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({
  invoke,
  convertFileSrc: (id: string) => `http://xf-preview.localhost/${id}`,
}));
let intersect: (visible: boolean) => void;
let resize: (width: number) => void;
const disconnect = vi.fn();
let objectUrls = 0;
const revoked: string[] = [];
beforeEach(() => {
  invoke.mockReset();
  disconnect.mockClear();
  clearStoredThumbnails();
  objectUrls = 0;
  revoked.length = 0;
  URL.createObjectURL = vi.fn(() => `blob:thumb-${(objectUrls += 1)}`);
  URL.revokeObjectURL = vi.fn((url: string) => void revoked.push(url));
  vi.stubGlobal(
    "IntersectionObserver",
    class {
      constructor(callback: (entries: object[]) => void) {
        intersect = (visible) => callback([{ isIntersecting: visible }]);
      }
      observe() {}
      disconnect = disconnect;
    },
  );
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(callback: (entries: object[]) => void) {
        resize = (width) => callback([{ contentRect: { width } }]);
      }
      observe() {}
      disconnect = disconnect;
    },
  );
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});
const document = {
  version: 1,
  id: "r1",
  title: "Report",
  description: null,
  page: "index.html",
  pages: ["index.html"],
  document_id: "doc1",
};

it("opens only visible reports and renders their isolated non-interactive document at card scale", async () => {
  invoke.mockResolvedValue(document);
  const view = render(
    <section>
      <ReportThumbnail reportId="r1" updatedAt={1n} thumbnailAt={null} />
    </section>,
  );
  expect(invoke).not.toHaveBeenCalled();
  act(() => {
    resize(300);
    intersect(true);
  });
  const frame = await screen.findByTitle("Report thumbnail");
  expect(invoke).toHaveBeenCalledWith("report_open", { reportId: "r1", page: null });
  expect(frame).toHaveAttribute("src", "http://xf-preview.localhost/doc1");
  expect(frame).toHaveAttribute("sandbox", "allow-scripts");
  expect(frame).toHaveAttribute("referrerpolicy", "no-referrer");
  expect(frame).toHaveAttribute("tabindex", "-1");
  expect(frame).toHaveAttribute("scrolling", "no");
  expect(frame.parentElement).toHaveAttribute("inert");
  expect(frame).toHaveStyle({ transform: "scale(0.25)", opacity: "0" });
  fireEvent.load(frame);
  expect(frame).toHaveStyle({ opacity: "1" });
  act(() => resize(600));
  expect(frame).toHaveStyle({ transform: "scale(0.5)" });
  expect(invoke).toHaveBeenCalledTimes(1);
  act(() => intersect(false));
  expect(screen.queryByTitle("Report thumbnail")).not.toBeInTheDocument();
  view.unmount();
  expect(disconnect).toHaveBeenCalledTimes(2);
});

it("ignores a document returned after the card leaves the viewport", async () => {
  let resolve!: (value: typeof document) => void;
  invoke.mockReturnValue(
    new Promise((done) => {
      resolve = done;
    }),
  );
  render(<ReportThumbnail reportId="r1" updatedAt={1n} thumbnailAt={null} />);
  act(() => {
    resize(300);
    intersect(true);
  });
  // The card dwells on screen long enough to start the fetch, then leaves.
  await waitFor(() => expect(invoke).toHaveBeenCalledTimes(1));
  act(() => intersect(false));
  await act(async () => resolve(document));
  expect(screen.queryByTitle("Report thumbnail")).not.toBeInTheDocument();
});

it("does not fetch a thumbnail for a card that only flies past during a fast scroll", async () => {
  render(<ReportThumbnail reportId="r1" updatedAt={1n} thumbnailAt={null} />);
  act(() => {
    resize(300);
    intersect(true);
  });
  // Gone again well inside the dwell, as in a fast scroll through a long library.
  act(() => intersect(false));
  await new Promise((done) => setTimeout(done, 300));
  expect(invoke).not.toHaveBeenCalled();
  expect(screen.queryByTitle("Report thumbnail")).not.toBeInTheDocument();
});

it("refuses a mismatched identity and leaves an explicit unavailable placeholder on failure", async () => {
  invoke.mockResolvedValue({ ...document, id: "other-account-report" });
  render(<ReportThumbnail reportId="r1" updatedAt={1n} thumbnailAt={null} />);
  act(() => {
    resize(300);
    intersect(true);
  });
  expect(await screen.findByText("Preview unavailable")).toBeInTheDocument();
  expect(screen.queryByTitle("Report thumbnail")).not.toBeInTheDocument();
});

it("resolves the current saved document again when its version changes", async () => {
  invoke.mockResolvedValue(document);
  const view = render(<ReportThumbnail reportId="r1" updatedAt={1n} thumbnailAt={null} />);
  act(() => {
    resize(300);
    intersect(true);
  });
  await screen.findByTitle("Report thumbnail");
  invoke.mockRejectedValue(new Error("unavailable"));
  view.rerender(<ReportThumbnail reportId="r1" updatedAt={2n} thumbnailAt={null} />);
  await waitFor(() => expect(invoke).toHaveBeenCalledTimes(2));
  expect(await screen.findByText("Preview unavailable")).toBeInTheDocument();
});

// "\x89PNG" as base64.
const stored = { version: 1, id: "r1", format: "png", data_base64: "iVBORw==" };

/** Answers each command the way a test names, and fails any other. */
function answer(byCommand: Record<string, () => unknown>) {
  invoke.mockImplementation((command: string) => {
    const reply = byCommand[command];
    return reply === undefined ? Promise.reject(new Error(`unexpected ${command}`)) : Promise.resolve().then(reply);
  });
}

function showOnScreen(thumbnailAt: bigint | null) {
  const view = render(<ReportThumbnail reportId="r1" updatedAt={1n} thumbnailAt={thumbnailAt} />);
  act(() => {
    resize(300);
    intersect(true);
  });
  return view;
}

it("shows DA's stored image and never opens the report itself", async () => {
  answer({ report_thumbnail: () => stored });
  const { container } = showOnScreen(9n);

  await waitFor(() => expect(container.querySelector("img")).toHaveAttribute("src", "blob:thumb-1"));
  const image = container.querySelector("img")!;
  expect(invoke).toHaveBeenCalledWith("report_thumbnail", { reportId: "r1" });
  expect(invoke).not.toHaveBeenCalledWith("report_open", expect.anything());
  expect(image).toHaveAttribute("alt", "");
  expect(image).toHaveAttribute("data-painted", "false");
  fireEvent.load(image);
  expect(image).toHaveAttribute("data-painted", "true");
  expect(screen.queryByTitle("Report thumbnail")).not.toBeInTheDocument();
});

it("keeps a stored image through scrolling, and paints a held one at once with no request", async () => {
  answer({ report_thumbnail: () => stored });
  const first = showOnScreen(9n);
  await waitFor(() => expect(first.container.querySelector("img")).not.toBeNull());
  fireEvent.load(first.container.querySelector("img")!);

  // Scrolled well away and back: the same picture, still painted, never asked for again.
  act(() => intersect(false));
  const image = first.container.querySelector("img");
  expect(image).toHaveAttribute("data-painted", "true");
  act(() => intersect(true));
  expect(first.container.querySelector("img")).toBe(image);
  first.unmount();

  // A card mounted later (another layout, a new list) paints the held image at once.
  const again = showOnScreen(9n);
  await waitFor(() => expect(again.container.querySelector("img")).not.toBeNull());
  expect(again.container.querySelector("img")).toHaveAttribute("data-painted", "true");
  expect(again.container.querySelector(".skeleton-shimmer")).toBeNull();
  expect(invoke).toHaveBeenCalledTimes(1);
});

it("keeps the old picture until a new version is ready", async () => {
  let resolveNew!: (value: unknown) => void;
  invoke.mockResolvedValueOnce(stored).mockReturnValueOnce(new Promise((done) => (resolveNew = done)));
  const view = showOnScreen(9n);
  await waitFor(() => expect(view.container.querySelector("img")).toHaveAttribute("src", "blob:thumb-1"));
  fireEvent.load(view.container.querySelector("img")!);

  view.rerender(<ReportThumbnail reportId="r1" updatedAt={1n} thumbnailAt={10n} />);
  await waitFor(() => expect(invoke).toHaveBeenCalledTimes(2));
  expect(view.container.querySelector("img")).toHaveAttribute("src", "blob:thumb-1");
  await act(async () => resolveNew(stored));
  await waitFor(() => expect(view.container.querySelector("img")).toHaveAttribute("src", "blob:thumb-2"));
  expect(view.container.querySelector("img")).toHaveAttribute("data-painted", "true");
});

it("draws the report itself when DA turns out to hold no image", async () => {
  answer({
    report_thumbnail: () => Promise.reject({ version: 1, code: "not_found" }),
    report_open: () => document,
  });
  showOnScreen(9n);

  const frame = await screen.findByTitle("Report thumbnail");
  expect(frame).toHaveAttribute("src", "http://xf-preview.localhost/doc1");
  expect(invoke).toHaveBeenCalledTimes(2);
});

it("says the preview is unavailable when the image cannot be read, without opening the report", async () => {
  answer({ report_thumbnail: () => Promise.reject({ version: 1, code: "storage_unavailable" }) });
  showOnScreen(9n);

  expect(await screen.findByText("Preview unavailable")).toBeInTheDocument();
  expect(invoke).toHaveBeenCalledTimes(1);
  expect(invoke).toHaveBeenCalledWith("report_thumbnail", { reportId: "r1" });
});

it("asks DA once per image version and lets an old version go", async () => {
  answer({ report_thumbnail: () => stored });

  await expect(storedThumbnailUrl("r1", 9n)).resolves.toBe("blob:thumb-1");
  await expect(storedThumbnailUrl("r1", 9n)).resolves.toBe("blob:thumb-1");
  expect(invoke).toHaveBeenCalledTimes(1);

  await expect(storedThumbnailUrl("r1", 10n)).resolves.toBe("blob:thumb-2");
  expect(invoke).toHaveBeenCalledTimes(2);
  await waitFor(() => expect(revoked).toEqual(["blob:thumb-1"]));

  // Sign-out lets every image go.
  clearStoredThumbnails();
  await waitFor(() => expect(revoked).toEqual(["blob:thumb-1", "blob:thumb-2"]));
});

it("refuses an image answered for another report and asks again next time", async () => {
  invoke.mockResolvedValueOnce({ ...stored, id: "other-account-report" }).mockResolvedValueOnce(stored);

  await expect(storedThumbnailUrl("r1", 9n)).rejects.toThrow();
  await expect(storedThumbnailUrl("r1", 9n)).resolves.toBe("blob:thumb-1");
  expect(invoke).toHaveBeenCalledTimes(2);
});

it("paints a held image on the first frame of a card that comes back, before it is seen to scroll in", async () => {
  answer({ report_thumbnail: () => stored });
  const first = showOnScreen(9n);
  await waitFor(() => expect(first.container.querySelector("img")).not.toBeNull());
  first.unmount();

  // A search cleared brings the card back: no observer report, no dwell, no request.
  const again = render(<ReportThumbnail reportId="r1" updatedAt={1n} thumbnailAt={9n} />);
  expect(again.container.querySelector("img")).toHaveAttribute("src", "blob:thumb-1");
  expect(invoke).toHaveBeenCalledTimes(1);
});

it("draws a report it has opened before from the same document, without asking DA again", async () => {
  invoke.mockResolvedValue(document);
  const first = render(<ReportThumbnail reportId="r1" updatedAt={1n} thumbnailAt={null} />);
  act(() => {
    resize(300);
    intersect(true);
  });
  await screen.findByTitle("Report thumbnail");
  first.unmount();

  render(<ReportThumbnail reportId="r1" updatedAt={1n} thumbnailAt={null} />);
  act(() => resize(300));
  expect(screen.getByTitle("Report thumbnail")).toHaveAttribute("src", "http://xf-preview.localhost/doc1");
  expect(invoke).toHaveBeenCalledTimes(1);

  // Sign-out forgets it: the next account asks for its own.
  clearStoredThumbnails();
  cleanup();
  render(<ReportThumbnail reportId="r1" updatedAt={1n} thumbnailAt={null} />);
  act(() => resize(300));
  expect(screen.queryByTitle("Report thumbnail")).not.toBeInTheDocument();
});
