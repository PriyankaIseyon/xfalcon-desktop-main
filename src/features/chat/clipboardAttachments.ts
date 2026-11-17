// Keep in step with uploads::kind: DA accepts at most 10 MiB for every chat file.
export const MAX_CHAT_ATTACHMENT_BYTES = 10 * 1024 * 1024;
export const MAX_PASTED_FILES = 3;
const EXTENSIONS = new Set([
  "png",
  "jpg",
  "jpeg",
  "gif",
  "webp",
  "csv",
  "tsv",
  "xlsx",
  "xls",
  "pptx",
  "ppt",
  "docx",
  "doc",
  "json",
  "txt",
  "md",
  "markdown",
  "sql",
  "py",
  "ts",
  "tsx",
  "js",
  "jsx",
  "rs",
  "go",
  "java",
  "yml",
  "yaml",
  "toml",
  "xml",
  "html",
  "css",
  "sh",
  "r",
  "log",
]);

export function clipboardFiles(clipboard: DataTransfer): File[] {
  // Some WebViews expose FileList, others only file items. Never read both:
  // they normally describe the same files.
  const files = Array.from(clipboard.files ?? []);
  if (files.length) return files;
  return Array.from(clipboard.items ?? [])
    .filter((item) => item.kind === "file")
    .map((item) => item.getAsFile())
    .filter((file): file is File => file !== null);
}

export function validateClipboardFiles(files: File[]): void {
  if (files.length > MAX_PASTED_FILES) throw new Error("Paste up to 3 files at a time.");
  for (const file of files) {
    if (file.size > MAX_CHAT_ATTACHMENT_BYTES) throw new Error("Each attachment must be 10 MiB or smaller.");
    if (file.size === 0) throw new Error("Empty files cannot be attached.");
    if (!EXTENSIONS.has(file.name.split(".").at(-1)?.toLowerCase() ?? "")) {
      throw new Error(
        "That file type cannot be attached. Use an image, Office document, spreadsheet, CSV, or text file.",
      );
    }
  }
}

export function clipboardFileBase64(file: File): Promise<string> {
  validateClipboardFiles([file]);
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error("That clipboard file could not be read."));
    reader.onabort = () => reject(new Error("Reading the clipboard file was cancelled."));
    reader.onload = () => {
      const result = reader.result;
      if (typeof result !== "string" || !result.includes(",")) {
        reject(new Error("That clipboard file could not be read."));
        return;
      }
      resolve(result.slice(result.indexOf(",") + 1));
    };
    reader.readAsDataURL(file);
  });
}
