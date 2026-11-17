import { describe, expect, it } from "vitest";
import { splitMarkdown } from "../../src/features/chat/richContent/splitMarkdown";

/**
 * The parse budget for a streaming answer.
 *
 * Splitting a message into its top-level blocks is what lets a settled fence
 * stop re-rendering, and re-parsing the WHOLE message to do it is quadratic over
 * a stream. That cost was measured, not assumed: parsing whole cost 261 ms for a
 * 3.7 KB answer in 80 steps, 1090 ms for 12.6 KB in 260, and 4521 ms for 25.4 KB
 * in 520 — doubling the answer and its step count quadrupled the bill, and by
 * 25 KB the parse dominated every other per-frame cost.
 *
 * The gate is a RATIO, not a duration: cost per byte must stay flat as the answer
 * grows. An absolute ceiling in jsdom would either flake on a loaded machine or
 * pass a quadratic implementation on a fast one.
 */

function answer(sections: number): string {
  const parts: string[] = [];
  for (let index = 0; index < sections; index += 1) {
    parts.push(`## Section ${index + 1}`, "");
    parts.push(`Paragraph ${index + 1} explains the query below with some **bold** and \`inline\` text.`, "");
    parts.push("```sql");
    for (let line = 0; line < 8; line += 1) parts.push(`select col_${index}_${line} from t_${index};`);
    parts.push("```", "");
  }
  return parts.join("\n");
}

/** Streams `steps` prefixes through the incremental path, as the renderer does. */
function streamIncremental(source: string, steps: number): number {
  let carried = splitMarkdown(source.slice(0, 1));
  const started = performance.now();
  for (let step = 1; step <= steps; step += 1) {
    carried = splitMarkdown(source.slice(0, Math.ceil((source.length * step) / steps)), carried);
  }
  return performance.now() - started;
}

/**
 * How much worse cost-per-byte may get as the answer grows fourfold.
 *
 * A linear split holds this near 1; measured 0.41. The whole-message split
 * measured 4.3 on the same fixtures, because each of four times as many steps
 * parses four times as much text — so this one number discriminates the two
 * implementations on its own.
 *
 * A ratio between two sizes of the SAME implementation, deliberately: a ratio
 * between two implementations, or any absolute jsdom ceiling, moves with machine
 * load, and a gate that flakes is a gate that gets deleted. The absolute figures
 * (4077 ms whole against 93 ms incremental at 25 KB) are recorded in
 * `benchmarks/chat/README.md` instead of asserted here.
 */
const MAX_COST_PER_BYTE_GROWTH = 1.8;

describe("markdown split budget", () => {
  it("keeps cost per byte flat as the answer grows", () => {
    const small = answer(20);
    const large = answer(80);
    const smallMs = streamIncremental(small, 130);
    const largeMs = streamIncremental(large, 520);
    const smallPerByte = smallMs / small.length;
    const largePerByte = largeMs / large.length;
    const growth = largePerByte / smallPerByte;

    console.log(
      `[split] small=${small.length}B ${smallMs.toFixed(0)}ms  large=${large.length}B ` +
        `${largeMs.toFixed(0)}ms  growth=${growth.toFixed(2)}x`,
    );
    expect(growth).toBeLessThan(MAX_COST_PER_BYTE_GROWTH);
  });
});
