//! How much of one tool result the model sees, sized to the model the run is
//! on (spec: agent-context-budget).
//!
//! One number per run, derived once from the model's context window and then
//! used everywhere a tool result is shaped for the model: the rendered outcome
//! the model reads, the durable preview stored on DA, a file page, a skill
//! page, an exact-result range. They have to agree — a page sized for a larger
//! budget than the renderer allows would be cut on the way out, and the cursor
//! it carried would then point past lines the model never saw.

/// One tool result may take this fraction of the window: 1/20, or 5%. A run
/// spends many rounds and parallel calls inside one window, so a single result
/// taking a large share of it would force compaction after a handful of
/// calls; 5% leaves room for twenty full-size results before that.
const WINDOW_SHARE_DIVISOR: u64 = 20;

/// The average rate provider tokenizers land at over mixed prose, code, and
/// JSON. An estimate, deliberately: the separate context admission check uses
/// a more conservative text estimate and then anchors on provider usage.
const BYTES_PER_TOKEN: u64 = 4;

/// Room the rendered outcome's envelope needs around the result it carries:
/// status, digest, byte length, the retrieval hint, and up to 64 content refs.
const ENVELOPE_RESERVE_BYTES: usize = 4 * 1024;

/// Room a page's own fields need beside its text: kind, line numbers, the
/// continuation hint. The path is charged separately, at its escaped length.
const PAGE_FIELDS_RESERVE_BYTES: usize = 1024;

/// Room an exact-result range's descriptor needs beside its base64 data.
const RANGE_FIELDS_RESERVE_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolResultBudget {
    bytes: usize,
}

impl ToolResultBudget {
    /// A 128k-token model's share, and what every model got before budgets
    /// scaled. No run gets less.
    pub const FLOOR_BYTES: usize = 32 * 1024;
    /// The largest preview DA stores (`MAX_PREVIEW_BYTES` in data-analyst's
    /// `models/context.py`, and the `tool_result_payloads` CHECK). A preview
    /// past it is refused when the run records its result.
    pub const CEILING_BYTES: usize = 256 * 1024;

    /// The budget for a run on a model with a `context_tokens`-token window.
    #[must_use]
    pub fn for_context_tokens(context_tokens: u64) -> Self {
        let scaled = (context_tokens / WINDOW_SHARE_DIVISOR).saturating_mul(BYTES_PER_TOKEN);
        let bytes = usize::try_from(scaled)
            .unwrap_or(usize::MAX)
            .clamp(Self::FLOOR_BYTES, Self::CEILING_BYTES);
        Self { bytes }
    }

    /// The smallest budget: what a caller with no model in hand must assume.
    #[must_use]
    pub const fn floor() -> Self {
        Self {
            bytes: Self::FLOOR_BYTES,
        }
    }

    /// The largest budget: what any stored preview may have been cut to.
    #[must_use]
    pub const fn ceiling() -> Self {
        Self {
            bytes: Self::CEILING_BYTES,
        }
    }

    /// The whole rendered outcome, envelope included.
    #[must_use]
    pub const fn bytes(self) -> usize {
        self.bytes
    }

    /// The structured result inside the envelope — what a preview may fill.
    #[must_use]
    pub const fn result_bytes(self) -> usize {
        self.bytes - ENVELOPE_RESERVE_BYTES
    }

    /// Text one page may carry, measured with [`json_string_len`], after its
    /// fields and a path of `path_len` escaped bytes.
    #[must_use]
    pub const fn page_bytes(self, path_len: usize) -> usize {
        self.result_bytes()
            .saturating_sub(PAGE_FIELDS_RESERVE_BYTES)
            .saturating_sub(path_len)
    }

    /// Raw bytes one exact-result range may return. The range travels as
    /// base64, four bytes for every three.
    #[must_use]
    pub const fn range_bytes(self) -> usize {
        self.result_bytes()
            .saturating_sub(RANGE_FIELDS_RESERVE_BYTES)
            / 4
            * 3
    }
}

/// The bytes `text` occupies inside a JSON string, quotes excluded.
///
/// A page is always shipped as JSON — to the model as a tool result, to the
/// webview over IPC — so a budget counted in raw UTF-8 bytes undercounts every
/// newline, quote, and backslash, and six-fold every other control character.
/// Counting the escaped form is what lets a page be exactly as large as its
/// budget and never larger.
#[must_use]
pub fn json_string_len(text: &str) -> usize {
    text.bytes()
        .map(|byte| match byte {
            b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 0x08 | 0x0c => 2,
            0x00..=0x1f => 6,
            _ => 1,
        })
        .sum()
}

/// The end of the longest prefix of `text` whose [`json_string_len`] is at
/// most `limit`, on a character boundary. How a page is cut to exactly its
/// budget.
#[must_use]
pub fn json_prefix_end(text: &str, limit: usize) -> usize {
    let mut used = 0;
    for (index, character) in text.char_indices() {
        let mut buffer = [0u8; 4];
        used += json_string_len(character.encode_utf8(&mut buffer));
        if used > limit {
            return index;
        }
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::{json_prefix_end, json_string_len, ToolResultBudget};

    #[test]
    fn the_budget_is_five_percent_of_the_window_at_four_bytes_a_token() {
        assert_eq!(
            ToolResultBudget::for_context_tokens(1_000_000).bytes(),
            200_000
        );
        assert_eq!(
            ToolResultBudget::for_context_tokens(500_000).bytes(),
            100_000
        );
        assert_eq!(
            ToolResultBudget::for_context_tokens(200_000).bytes(),
            40_000
        );
    }

    #[test]
    fn small_windows_get_the_floor_and_huge_ones_the_ceiling() {
        assert_eq!(
            ToolResultBudget::for_context_tokens(128_000).bytes(),
            ToolResultBudget::FLOOR_BYTES
        );
        assert_eq!(
            ToolResultBudget::for_context_tokens(0).bytes(),
            ToolResultBudget::FLOOR_BYTES
        );
        assert_eq!(
            ToolResultBudget::for_context_tokens(10_000_000).bytes(),
            ToolResultBudget::CEILING_BYTES
        );
        assert_eq!(
            ToolResultBudget::for_context_tokens(u64::MAX).bytes(),
            ToolResultBudget::CEILING_BYTES
        );
    }

    #[test]
    fn every_derived_size_fits_inside_the_one_it_is_carried_in() {
        for budget in [
            ToolResultBudget::floor(),
            ToolResultBudget::for_context_tokens(1_000_000),
            ToolResultBudget::for_context_tokens(u64::MAX),
        ] {
            assert!(budget.result_bytes() < budget.bytes());
            assert!(budget.page_bytes(4096) < budget.result_bytes());
            // Base64 of a full range, plus its fields, fits the result.
            assert!(budget.range_bytes().div_ceil(3) * 4 + 1024 <= budget.result_bytes());
        }
    }

    #[test]
    fn a_prefix_is_cut_to_its_escaped_budget_on_a_character_boundary() {
        assert_eq!(json_prefix_end("abcdef", 4), 4);
        assert_eq!(json_prefix_end("a\nb", 2), 1);
        assert_eq!(json_prefix_end("éé", 3), 2);
        assert_eq!(json_prefix_end("short", 100), 5);
    }

    #[test]
    fn escaped_length_counts_what_json_adds() {
        assert_eq!(json_string_len("plain"), 5);
        assert_eq!(json_string_len("a\nb"), 4);
        assert_eq!(json_string_len("\"\\"), 4);
        assert_eq!(json_string_len("\u{1}"), 6);
        assert_eq!(json_string_len("é"), 2);
        let sample = "line \"one\"\n\ttwo\\\u{7}é";
        let encoded = serde_json::to_string(sample).expect("serializes");
        assert_eq!(json_string_len(sample), encoded.len() - 2);
    }
}
