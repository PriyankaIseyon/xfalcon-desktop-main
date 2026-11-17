import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(fileURLToPath(import.meta.url));
const messages = Array.from({ length: 10_000 }, (_, index) => ({
  message_id: `m-${index}`,
  role: index % 2 === 0 ? "user" : "assistant",
  text:
    index % 17 === 0
      ? "```ts\nconst answer = 42;\n```\n\nShort **note**."
      : `Message ${index} with a modest amount of text for virtualization.`,
  sequence: index + 1,
}));

writeFileSync(join(root, "fixture-10k.json"), JSON.stringify({ messages }, null, 0));
