//! Saving one composed document to a place the person chose
//! (spec: lean-report-export).
//!
//! The document's text crosses the boundary; its destination never does. The
//! renderer composes the bytes from what it is already showing and hands them
//! over, and the backend opens the OS save dialog itself — so there is no
//! parameter a caller could aim at a path of its own choosing, and no write
//! that a person did not just approve by naming the file.
//!
//! One command for every "give me this as a file" action in the app, rather
//! than one per surface. A second path would be a second place for the
//! destination rule to be weaker, and `src-tauri/src/security/tests.rs` records
//! why the alternative — a `write_file` the renderer aims itself — was refused.
//!
//! Nothing here carries the chosen directory back. The renderer needs enough
//! to confirm what it wrote, which is the file's name; the absolute path would
//! hand the webview a map of the user's disk that it has no reason to hold.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{ContractVersion, CONTRACT_VERSION};

/// What kind of file is being saved.
///
/// The format decides the dialog's filter and the extension the backend
/// re-asserts, so a caller cannot promise one kind of file and write another.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum DocumentExportFormat {
    /// An exported conversation document.
    Markdown,
    /// A data grid's rows.
    Csv,
    /// A chart, as a standalone vector image.
    Svg,
    /// A model-authored HTML document, as its own file.
    Html,
}

impl DocumentExportFormat {
    /// The extension this format is always written under.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Csv => "csv",
            Self::Svg => "svg",
            Self::Html => "html",
        }
    }

    /// The save dialog's filter label.
    #[must_use]
    pub const fn filter_label(self) -> &'static str {
        match self {
            Self::Markdown => "Markdown",
            Self::Csv => "CSV",
            Self::Svg => "SVG image",
            Self::Html => "HTML document",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DocumentExportResult {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    /// `false` when the person dismissed the save dialog. A cancelled save is
    /// a decision, not a failure, so it comes back as a successful call whose
    /// answer is "nothing was written" — a caller that reported an error here
    /// would accuse the user of a fault they did not commit.
    pub saved: bool,
    /// The written file's name. `None` whenever `saved` is `false`.
    pub file_name: Option<String>,
}

impl DocumentExportResult {
    #[must_use]
    pub fn cancelled() -> Self {
        Self {
            version: CONTRACT_VERSION,
            saved: false,
            file_name: None,
        }
    }

    #[must_use]
    pub fn written(file_name: impl Into<String>) -> Self {
        Self {
            version: CONTRACT_VERSION,
            saved: true,
            file_name: Some(file_name.into()),
        }
    }
}
