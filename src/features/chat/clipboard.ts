export async function writeClipboardText(text: string): Promise<void> {
  await navigator.clipboard.writeText(text);
}

export function readClipboardText(clipboard: DataTransfer): string {
  const plain = clipboard.getData("text/plain");
  if (plain) return plain;
  const html = clipboard.getData("text/html");
  if (!html) return "";

  // Template contents are inert: pasted scripts, images and styles never enter
  // the document. Keep cell and paragraph boundaries in the plain-text prompt.
  const template = document.createElement("template");
  template.innerHTML = html;
  template.content
    .querySelectorAll("script, style, noscript, template, head, [hidden]")
    .forEach((node) => node.remove());
  const blocks = new Set([
    "P",
    "DIV",
    "LI",
    "UL",
    "OL",
    "TR",
    "TABLE",
    "H1",
    "H2",
    "H3",
    "H4",
    "H5",
    "H6",
    "BLOCKQUOTE",
    "PRE",
  ]);
  const textOf = (node: Node): string => {
    if (node.nodeType === Node.TEXT_NODE) return node.textContent ?? "";
    if (!(node instanceof Element)) return "";
    if (node.tagName === "BR") return "\n";
    const text = Array.from(node.childNodes, textOf).join("");
    if (node.tagName === "TD" || node.tagName === "TH") return `${text.trim()}\t`;
    return blocks.has(node.tagName) ? `\n${text}\n` : text;
  };
  return Array.from(template.content.childNodes, textOf)
    .join("")
    .replace(/\r\n?/g, "\n")
    .replace(/[\t ]+\n/g, "\n")
    .replace(/\n{2,}/g, "\n")
    .trim();
}
