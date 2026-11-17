//! Search over the incremental project index (`super::index`), plus
//! bounded, streaming content search over the trusted filesystem for the
//! files the index says are text.
//!
//! Only [`search_files`]/[`search_text`] read from the index;
//! `super::read`'s direct reads/listing never do (see that module's doc
//! comment). The index is populated incrementally and non-blockingly
//! (`super::index`), so both functions here return whatever is indexed so
//! far and never block on, or require, indexing to have completed --
//! `IndexStore::list_all`'s `truncated`/index-population state is
//! propagated as `index_truncated` rather than treated as an error.
//!
//! # Deterministic ranking
//!
//! Results are ordered by an explicit tie-break key, never by
//! hash-map/SQL/filesystem enumeration order -- matching
//! `crate::tools::registry::ToolRegistry` and
//! `crate::skills::registry::SkillRegistry`'s own "deterministic
//! enumeration" discipline (both use `IndexMap` for that reason; search
//! results have no natural insertion order to preserve, so this module
//! defines an explicit sort key instead: [`search_files`] ranks by match
//! tier (exact basename, basename prefix, basename substring, path
//! substring) then lexical path; [`search_text`] scans candidate files in
//! lexical path order, so which files are scanned first -- and therefore
//! which matches survive [`MAX_SEARCH_FILES_SCANNED`]/[`MAX_SEARCH_RESULTS`]
//! -- is itself deterministic and testable).
//!
//! # Sensitive files never enter content search without a matching override
//!
//! [`search_text`] silently excludes (never errors on) a classified-sensitive
//! candidate unless the caller presents a [`SensitiveOverrideGrant`] whose
//! scope matches that exact file -- the same default-deny contract
//! `super::read::read_file` enforces for a single explicit read, applied
//! per-candidate here. [`search_files`] has no such restriction: it never
//! returns file *content*, only paths/metadata, each labeled
//! `is_sensitive` so a caller can flag it -- matching
//! `super::read::list_directory`'s identical "names are not content"
//! posture.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use xfalcon_contracts::RunId;

use super::domain::{ProjectId, TrustedRoot};
use super::index::{project_walk_builder, IndexError, IndexPolicy, IndexStore, IndexedFileRecord};
use super::path_policy::RelativeProjectPath;
use super::read::{sniff_content_kind, ContentKind, SNIFF_BYTES};
use super::sensitive::{SecretClassifier, SensitiveOverrideGrant, SensitiveReadScope};
use crate::security::account_cleanup::ValidatedAccountId;

pub const MAX_SEARCH_QUERY_BYTES: usize = 512;
pub const MAX_SEARCH_RESULTS: usize = 200;
pub const MAX_SEARCH_FILES_SCANNED: usize = 5_000;
pub const MAX_MATCHES_PER_FILE: usize = 20;
pub const MAX_SEARCH_BYTES_PER_FILE: u64 = 4 * 1024 * 1024;
pub const MAX_MATCH_LINE_PREVIEW_BYTES: usize = 500;
/// Hard ceiling on the compiled size of a caller-supplied regex, and on its
/// lazy-DFA cache. A pattern is untrusted input (TM-006): this bounds what a
/// pathological one can allocate, so it fails to compile instead of exhausting
/// memory. The `regex` crate has no backtracking, so there is no
/// catastrophic-backtracking class to bound here -- linear-time matching is a
/// property of the engine, not of the pattern.
pub const MAX_PATTERN_PROGRAM_BYTES: usize = 1024 * 1024;
/// Hard ceiling on files visited by the live walk that supplements the index.
pub const MAX_LIVE_WALK_FILES: usize = 20_000;
pub const MAX_OFFICE_FILES_PER_SEARCH: usize = 32;
pub const MAX_OFFICE_SOURCE_BYTES_PER_SEARCH: u64 = 128 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("search query must not be empty")]
    EmptyQuery,
    #[error("search query exceeds {0} bytes")]
    QueryTooLong(usize),
    // The crate's own message names the position and the problem, so it is
    // passed through verbatim: the caller is about to correct the pattern.
    #[error("the search pattern is not a valid regular expression: {0}")]
    InvalidRegex(String),
    #[error("the search pattern is not a valid glob: {0}")]
    InvalidGlob(String),
    #[error(transparent)]
    Index(#[from] IndexError),
    #[error(transparent)]
    Containment(#[from] super::domain::ContainmentError),
    #[error("io error while searching a trusted-project file: {0}")]
    Io(#[from] std::io::Error),
}

fn validate_query(query: &str) -> Result<(), SearchError> {
    if query.is_empty() {
        return Err(SearchError::EmptyQuery);
    }
    if query.len() > MAX_SEARCH_QUERY_BYTES {
        return Err(SearchError::QueryTooLong(MAX_SEARCH_QUERY_BYTES));
    }
    Ok(())
}

/// How a content query is interpreted. Literal is the default because most
/// searches are for an identifier, and silently reinterpreting `a.b` or `foo(`
/// as a pattern would make an existing literal search mean something else.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SearchMode {
    #[default]
    Literal,
    Regex,
}

/// How a path query is interpreted. Substring ranking is the default for the
/// same reason literal is the content default.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FileSearchMode {
    #[default]
    Substring,
    Glob,
}

/// A compiled content query. Built once per search, then applied per line, so
/// a regex is never recompiled inside the scan loop.
enum Matcher {
    Literal {
        needle: String,
        case_sensitive: bool,
    },
    Regex(regex::Regex),
}

impl Matcher {
    fn compile(query: &str, mode: SearchMode, case_sensitive: bool) -> Result<Self, SearchError> {
        match mode {
            SearchMode::Literal => Ok(Self::Literal {
                needle: if case_sensitive {
                    query.to_owned()
                } else {
                    query.to_lowercase()
                },
                case_sensitive,
            }),
            SearchMode::Regex => regex::RegexBuilder::new(query)
                .case_insensitive(!case_sensitive)
                .size_limit(MAX_PATTERN_PROGRAM_BYTES)
                .dfa_size_limit(MAX_PATTERN_PROGRAM_BYTES)
                .build()
                .map(Self::Regex)
                .map_err(|error| SearchError::InvalidRegex(error.to_string())),
        }
    }

    fn is_match(&self, line: &str) -> bool {
        match self {
            Self::Literal {
                needle,
                case_sensitive,
            } => {
                if *case_sensitive {
                    line.contains(needle.as_str())
                } else {
                    line.to_lowercase().contains(needle.as_str())
                }
            }
            Self::Regex(pattern) => pattern.is_match(line),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileSearchResult {
    pub relative_path: RelativeProjectPath,
    pub is_sensitive: bool,
    pub content_kind: ContentKind,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileSearchOutcome {
    pub results: Vec<FileSearchResult>,
    /// Set when more matches existed than [`MAX_SEARCH_RESULTS`] allows.
    pub truncated: bool,
    /// Set when the underlying index itself holds more rows than
    /// `IndexStore::list_all` returned (see [`super::index::MAX_INDEXED_FILES_PER_PASS`]);
    /// distinct from `truncated`, which is about *this query's* result
    /// count.
    pub index_truncated: bool,
    /// Files the live walk visited beyond what the index already listed. Lets
    /// a caller tell "nothing matched" from "the index was cold", and pairs
    /// with `truncated` to tell either from "not everything was looked at".
    pub live_scanned: usize,
}

/// Filename search: ranks every indexed path by how `query` matches its
/// basename or full path (see the module doc comment's tier ordering).
/// Never touches the filesystem -- pure computation over what
/// `IndexStore::list_all` already returned.
pub fn search_files(
    trusted_root: &TrustedRoot,
    store: &IndexStore,
    classifier: &SecretClassifier,
    account: &ValidatedAccountId,
    project_id: ProjectId,
    query: &str,
    mode: FileSearchMode,
) -> Result<FileSearchOutcome, SearchError> {
    validate_query(query)?;
    let (records, index_truncated) = store.list_all(account, project_id)?;
    let matcher = PathMatcher::compile(query, mode)?;

    // The index is a fast path, not the source of truth: a file it has not
    // reached yet is still on disk, and a search that cannot see it answers
    // "no matches" for a file a direct read would return.
    let indexed: BTreeSet<String> = records
        .iter()
        .map(|record| record.relative_path.as_str().to_owned())
        .collect();
    let live = live_candidates(trusted_root, &indexed)?;

    // Ranking is path-only, so it stays cheap over the whole candidate set.
    // The per-file details a live candidate needs are filled in *after*
    // truncation, which bounds the stat-and-sniff work by the result count
    // rather than by how many files matched.
    let mut ranked: Vec<(RankKey, Ranked<'_>)> = records
        .iter()
        .filter_map(|record| {
            matcher
                .rank(record.relative_path.as_str())
                .map(|rank| (rank, Ranked::Indexed(record)))
        })
        .chain(live.paths.iter().filter_map(|path| {
            matcher
                .rank(path.as_str())
                .map(|rank| (rank, Ranked::Live(path)))
        }))
        .collect();

    ranked.sort_by(|(a, _), (b, _)| a.cmp(b));
    let truncated = ranked.len() > MAX_SEARCH_RESULTS;
    ranked.truncate(MAX_SEARCH_RESULTS);

    let results = ranked
        .into_iter()
        .filter_map(|(_, candidate)| match candidate {
            Ranked::Indexed(record) => Some(FileSearchResult {
                relative_path: record.relative_path.clone(),
                is_sensitive: record.is_sensitive,
                content_kind: record.content_kind,
                size_bytes: record.size_bytes,
            }),
            Ranked::Live(path) => live_result(trusted_root, classifier, path),
        })
        .collect();

    Ok(FileSearchOutcome {
        results,
        truncated,
        index_truncated,
        live_scanned: live.scanned,
    })
}

/// A matched path, before the details of the result are filled in.
enum Ranked<'a> {
    Indexed(&'a IndexedFileRecord),
    Live(&'a RelativeProjectPath),
}

/// Describes a live candidate the way the index would have: size from a stat,
/// content kind from the same bounded prefix sniff a partial read uses, and
/// sensitivity from the path alone. `None` when the file has gone since the
/// walk saw it, which is a stale candidate rather than an error for the call.
fn live_result(
    trusted_root: &TrustedRoot,
    classifier: &SecretClassifier,
    path: &RelativeProjectPath,
) -> Option<FileSearchResult> {
    let resolved = trusted_root.resolve_existing(path.clone()).ok()?;
    let size_bytes = trusted_root.file_metadata(&resolved).ok()?.len();
    let is_sensitive = classifier.classify(path).is_some();
    // A sensitive path is never opened, not even to classify its content --
    // the decision is made from the path before the file is touched.
    let content_kind = if is_sensitive {
        ContentKind::Text
    } else {
        sniff_kind(trusted_root, &resolved).unwrap_or(ContentKind::Text)
    };
    Some(FileSearchResult {
        relative_path: path.clone(),
        is_sensitive,
        content_kind,
        size_bytes,
    })
}

fn sniff_kind(
    trusted_root: &TrustedRoot,
    resolved: &super::domain::ResolvedProjectPath,
) -> Option<ContentKind> {
    let mut file = trusted_root.open_file(resolved).ok()?;
    let mut prefix = vec![0u8; SNIFF_BYTES];
    let read = file.read(&mut prefix).ok()?;
    prefix.truncate(read);
    Some(sniff_content_kind(&prefix))
}

/// Paths found on disk that the index did not already list.
struct LiveWalk {
    paths: Vec<RelativeProjectPath>,
    scanned: usize,
}

/// Walks the project for files the index has not reached yet, using the same
/// configuration an index pass uses (`super::index::project_walk_builder`), so
/// "found on disk" and "found in the index" cannot mean different sets of
/// files. Bounded by [`MAX_LIVE_WALK_FILES`].
///
/// Enumerates paths only -- no content is read and nothing is classified here.
/// A caller decides sensitivity from the path (cheap, and the same default-deny
/// decision `super::sensitive` makes for an indexed candidate) and reads
/// content only for the candidates it actually keeps.
fn live_candidates(
    trusted_root: &TrustedRoot,
    already_indexed: &BTreeSet<String>,
) -> Result<LiveWalk, SearchError> {
    let root_path = trusted_root.canonical_path().to_path_buf();
    let mut paths = Vec::new();
    let mut scanned = 0usize;

    for entry in project_walk_builder(&root_path, &IndexPolicy::default())
        .build()
        .flatten()
    {
        if scanned >= MAX_LIVE_WALK_FILES {
            break;
        }
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let Ok(suffix) = entry.path().strip_prefix(&root_path) else {
            continue;
        };
        let Some(relative) = suffix.to_str() else {
            continue;
        };
        // Normalized to the one separator every project-relative path in this
        // module tree uses, so a Windows walk and an index row compare equal.
        let relative = relative.replace('\\', "/");
        scanned += 1;
        if already_indexed.contains(&relative) {
            continue;
        }
        // A path the policy itself rejects is not a candidate; the walk found
        // it, but nothing here may act on it.
        if let Ok(path) = RelativeProjectPath::new(&relative) {
            paths.push(path);
        }
    }

    Ok(LiveWalk { paths, scanned })
}

/// A compiled path query: either the substring ranking this module has always
/// used, or a glob.
enum PathMatcher {
    Substring(String),
    Glob(globset::GlobMatcher),
}

impl PathMatcher {
    fn compile(query: &str, mode: FileSearchMode) -> Result<Self, SearchError> {
        match mode {
            FileSearchMode::Substring => Ok(Self::Substring(query.to_lowercase())),
            FileSearchMode::Glob => globset::GlobBuilder::new(query)
                // `*` must not cross a path separator, so `src/*.rs` means the
                // files directly in `src/` and `src/**/*.rs` means the subtree
                // -- the meaning anyone writing a glob expects.
                .literal_separator(true)
                .case_insensitive(true)
                .build()
                .map(|glob| Self::Glob(glob.compile_matcher()))
                .map_err(|error| SearchError::InvalidGlob(error.to_string())),
        }
    }

    /// `None` when the path does not match at all.
    fn rank(&self, path: &str) -> Option<RankKey> {
        match self {
            Self::Substring(query_lower) => rank_filename(path, query_lower),
            // A glob is a yes/no test, so every match shares one tier and the
            // lexical path is the whole ordering.
            Self::Glob(matcher) => matcher.is_match(path).then(|| RankKey {
                tier: 0,
                path: path.to_owned(),
            }),
        }
    }
}

/// Explicit, total tie-break ordering for filename search: lower `tier`
/// (better match quality) sorts first; `path` breaks ties lexically so two
/// results in the same tier always come back in the same order regardless
/// of the index's own row order.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd)]
struct RankKey {
    tier: u8,
    path: String,
}

fn rank_filename(path: &str, query_lower: &str) -> Option<RankKey> {
    let basename = Path::new(path)
        .file_name()?
        .to_string_lossy()
        .to_lowercase();
    let path_lower = path.to_lowercase();

    let tier = if basename == query_lower {
        0
    } else if basename.starts_with(query_lower) {
        1
    } else if basename.contains(query_lower) {
        2
    } else if path_lower.contains(query_lower) {
        3
    } else {
        return None;
    };
    Some(RankKey {
        tier,
        path: path.to_owned(),
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextMatch {
    pub relative_path: RelativeProjectPath,
    pub line_number: u32,
    pub line_preview: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextSearchOutcome {
    pub matches: Vec<TextMatch>,
    pub truncated: bool,
    pub index_truncated: bool,
    pub files_scanned: usize,
    /// Indexed candidates skipped this call because they could not actually
    /// be resolved/opened/scanned (deleted, renamed, or permission-changed
    /// since indexing ran, or another transient I/O failure) -- never fatal
    /// for the whole call, see [`search_text`]'s doc comment.
    pub files_stale: usize,
    /// Files the live walk visited beyond what the index already listed.
    pub live_scanned: usize,
}

/// Content search: scans indexed files the index already classified as
/// [`ContentKind::Text`] plus supported Office containers, whose bounded
/// extracted text is searched in lexical path order. Other binary,
/// non-UTF-8, and oversized files never enter content search. Plain text is
/// streamed line-by-line rather than loaded whole (`docs/threat-model.md`
/// TM-008); Office extraction is capped at 32 files and 128 MiB of source
/// bytes per call. A
/// classified-sensitive candidate is skipped entirely unless `overrides`
/// contains a grant matching that exact `(project, path, run)` -- see the
/// module doc comment.
///
/// A single stale candidate -- deleted, renamed, or permission-changed on
/// disk since `super::index` last observed it, or any other transient I/O
/// failure while resolving/opening/scanning it -- is skipped (counted in
/// [`TextSearchOutcome::files_stale`]), never treated as a hard error for
/// the whole call: the index is populated asynchronously and may always lag
/// the real filesystem by design (see the module doc comment's "return
/// whatever is indexed so far" contract), so this is routine, not
/// exceptional, and must not discard matches already found in earlier,
/// still-valid files. Mirrors `super::read::classify_child`'s identical
/// "not classifiable/readable rather than erroring the whole call over one
/// child" posture.
#[allow(clippy::too_many_arguments)]
pub fn search_text(
    trusted_root: &TrustedRoot,
    store: &IndexStore,
    classifier: &SecretClassifier,
    account: &ValidatedAccountId,
    project_id: ProjectId,
    query: &str,
    mode: SearchMode,
    case_sensitive: bool,
    overrides: &[SensitiveOverrideGrant],
    run_id: &RunId,
) -> Result<TextSearchOutcome, SearchError> {
    validate_query(query)?;
    let (mut records, index_truncated) = store.list_all(account, project_id)?;
    records.sort_by(|a, b| a.relative_path.as_str().cmp(b.relative_path.as_str()));

    let matcher = Matcher::compile(query, mode, case_sensitive)?;

    // Indexed candidates first (their content kind and sensitivity are already
    // known), then anything on disk the index has not reached -- see
    // `live_candidates`.
    let indexed: BTreeSet<String> = records
        .iter()
        .map(|record| record.relative_path.as_str().to_owned())
        .collect();
    let live = live_candidates(trusted_root, &indexed)?;
    let candidates: Vec<Candidate> = records
        .iter()
        .map(|record| Candidate {
            relative_path: record.relative_path.clone(),
            is_office: crate::documents::is_office_path(record.relative_path.as_path()),
            // Office files are extracted through the shared parser even when
            // the raw-byte index correctly classifies their containers as binary.
            skip: !matches!(record.content_kind, ContentKind::Text)
                && !crate::documents::is_office_path(record.relative_path.as_path()),
            is_sensitive: record.is_sensitive,
        })
        .chain(live.paths.iter().map(|path| Candidate {
            relative_path: path.clone(),
            is_office: crate::documents::is_office_path(path.as_path()),
            // Content kind is unknown until it is read; the scan itself skips
            // lines that are not valid UTF-8, so a binary file yields nothing
            // rather than raw bytes.
            skip: false,
            // Decided from the path alone, before the file is opened -- the
            // same default-deny decision an indexed candidate already carries.
            is_sensitive: classifier.classify(path).is_some(),
        }))
        .collect();

    let mut matches = Vec::new();
    let mut truncated = false;
    let mut files_scanned = 0usize;
    let mut files_stale = 0usize;
    let mut office_files_scanned = 0usize;
    let mut office_source_bytes = 0u64;

    'candidates: for record in &candidates {
        if files_scanned >= MAX_SEARCH_FILES_SCANNED {
            truncated = true;
            break;
        }
        if record.skip {
            continue;
        }
        if record.is_sensitive {
            let scope =
                SensitiveReadScope::new(project_id, record.relative_path.clone(), run_id.clone());
            if !overrides.iter().any(|grant| grant.matches(&scope)) {
                continue;
            }
        }

        files_scanned += 1;
        // A candidate the index still lists may have been deleted, renamed,
        // or had its permissions changed since indexing ran, or hit some
        // other transient I/O failure -- skip just this one stale candidate
        // rather than discarding every match already found in this call
        // (see this function's doc comment).
        let resolved = match trusted_root.resolve_existing(record.relative_path.clone()) {
            Ok(resolved) => resolved,
            Err(_) => {
                files_stale += 1;
                continue;
            }
        };
        let office_size = if record.is_office {
            if office_files_scanned >= MAX_OFFICE_FILES_PER_SEARCH {
                truncated = true;
                continue;
            }
            let metadata = match trusted_root.file_metadata(&resolved) {
                Ok(metadata) if metadata.is_file() => metadata,
                _ => {
                    files_stale += 1;
                    continue;
                }
            };
            let size = metadata.len();
            if size > crate::documents::MAX_DOCUMENT_BYTES {
                files_stale += 1;
                continue;
            }
            if size > MAX_OFFICE_SOURCE_BYTES_PER_SEARCH.saturating_sub(office_source_bytes) {
                truncated = true;
                continue;
            }
            Some(size)
        } else {
            None
        };
        let mut file = match trusted_root.open_file(&resolved) {
            Ok(file) => file,
            Err(_) => {
                files_stale += 1;
                continue;
            }
        };
        let file_matches = if record.is_office {
            let metadata = match file.metadata() {
                Ok(metadata) if metadata.is_file() => metadata,
                _ => {
                    files_stale += 1;
                    continue;
                }
            };
            let source_size = metadata.len();
            if Some(source_size) != office_size
                || source_size > crate::documents::MAX_DOCUMENT_BYTES
            {
                files_stale += 1;
                continue;
            }
            office_files_scanned += 1;
            let max_read = crate::documents::MAX_DOCUMENT_BYTES
                .min(MAX_OFFICE_SOURCE_BYTES_PER_SEARCH.saturating_sub(office_source_bytes));
            let mut bytes = Vec::with_capacity(source_size as usize);
            let read_result = file.by_ref().take(max_read).read_to_end(&mut bytes);
            office_source_bytes = office_source_bytes.saturating_add(bytes.len() as u64);
            if read_result.is_err() {
                files_stale += 1;
                continue;
            }
            let after_read = match file.metadata() {
                Ok(metadata) if metadata.is_file() => metadata,
                _ => {
                    files_stale += 1;
                    continue;
                }
            };
            if after_read.len() != source_size {
                files_stale += 1;
                continue;
            }
            if bytes.len() as u64 != source_size {
                files_stale += 1;
                continue;
            }
            match crate::documents::extract(record.relative_path.as_path(), &bytes) {
                Ok(extracted) => {
                    if extracted.text.as_bytes().contains(&0) {
                        files_stale += 1;
                        continue;
                    }
                    let (body, header_line_offset) = extracted.search_body();
                    scan_file_for_matches(std::io::Cursor::new(body.as_bytes()), &matcher).map(
                        |matches| {
                            matches
                                .into_iter()
                                .map(|(line, preview)| {
                                    (line.saturating_add(header_line_offset), preview)
                                })
                                .collect()
                        },
                    )
                }
                // A structurally valid blank or image-only document has no
                // searchable text, but the indexed source is still current.
                Err(crate::documents::DocumentError::NoText) => Ok(Vec::new()),
                Err(_) => {
                    files_stale += 1;
                    continue;
                }
            }
        } else {
            scan_file_for_matches(file, &matcher)
        };
        let file_matches = match file_matches {
            Ok(file_matches) => file_matches,
            Err(_) => {
                files_stale += 1;
                continue;
            }
        };
        for (line_number, line_preview) in file_matches {
            if matches.len() >= MAX_SEARCH_RESULTS {
                truncated = true;
                break 'candidates;
            }
            matches.push(TextMatch {
                relative_path: record.relative_path.clone(),
                line_number,
                line_preview,
            });
        }
    }

    Ok(TextSearchOutcome {
        matches,
        truncated,
        index_truncated,
        files_scanned,
        files_stale,
        live_scanned: live.scanned,
    })
}

/// One file content search will consider, from either source.
struct Candidate {
    relative_path: RelativeProjectPath,
    is_office: bool,
    /// Already known not to be text, so it never enters content search.
    skip: bool,
    is_sensitive: bool,
}

/// Streams `file` line-by-line applying `matcher`, bounded by
/// [`MAX_SEARCH_BYTES_PER_FILE`] total, [`MAX_MATCHES_PER_FILE`] matches,
/// and [`MAX_MATCH_LINE_PREVIEW_BYTES`] per returned preview. A line that is
/// not valid UTF-8 is skipped (not an error for the whole file) -- content
/// search over a file the index already classified as `Text` should not
/// normally hit this, but a later byte the index's own classification
/// bound (`super::read::MAX_HASHED_FILE_BYTES`) never inspected could still
/// be invalid; skipping it is the same defense-in-depth posture
/// `super::read::read_partial` takes.
fn scan_file_for_matches(
    file: impl Read,
    matcher: &Matcher,
) -> Result<Vec<(u32, String)>, SearchError> {
    let mut reader = BufReader::new(file);
    let mut matches = Vec::new();
    let mut line_number: u32 = 0;
    let mut total_bytes: u64 = 0;
    let mut raw_line = Vec::new();

    loop {
        raw_line.clear();
        let read = reader.read_until(b'\n', &mut raw_line)?;
        if read == 0 {
            break;
        }
        line_number += 1;
        total_bytes += read as u64;
        if total_bytes > MAX_SEARCH_BYTES_PER_FILE {
            break;
        }
        if raw_line.last() == Some(&b'\n') {
            raw_line.pop();
            if raw_line.last() == Some(&b'\r') {
                raw_line.pop();
            }
        }
        let Ok(text) = std::str::from_utf8(&raw_line) else {
            continue;
        };

        if matcher.is_match(text) {
            let mut preview = text.to_owned();
            truncate_at_char_boundary(&mut preview, MAX_MATCH_LINE_PREVIEW_BYTES);
            matches.push((line_number, preview));
            if matches.len() >= MAX_MATCHES_PER_FILE {
                break;
            }
        }
    }
    Ok(matches)
}

/// Truncates `text` to at most `max_bytes` bytes without splitting a
/// multi-byte UTF-8 character (plain `String::truncate` panics if the given
/// index is not a char boundary).
fn truncate_at_char_boundary(text: &mut String, max_bytes: usize) {
    if text.len() <= max_bytes {
        return;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
}
