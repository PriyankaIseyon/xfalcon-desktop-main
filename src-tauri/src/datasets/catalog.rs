//! The datasets a person has in tenant storage, as the destination reads them.
//!
//! Four operations over DA's upload endpoints — list, detail, delete, download
//! — plus the two rules that make the list trustworthy:
//!
//! - **Only `ready` rows are listed.** An `uploading` row whose transfer died
//!   is not a dataset anyone can query, and a `failed` one never will be.
//!   Listing either as a permanent entry would make the catalog a record of
//!   attempts rather than of what exists.
//! - **A truncated list says so.** DA paginates; this walks to exhaustion, and
//!   if the page cap stops the walk first the surface is told. A partial list
//!   presented as complete is how a person concludes their dataset was lost.
//!
//! Timestamps are converted here. DA sends ISO-8601 strings; every surface
//! reads epoch milliseconds, so there is one parser rather than one per
//! renderer.

use std::path::Path;

use xfalcon_contracts::datasets::{
    DatasetColumnView, DatasetDetail, DatasetErrorCode, DatasetFileView, DatasetId,
    DatasetListResult, DatasetStatus, DatasetView,
};

use crate::clients::da::generated::{DatasetDetailResponse, DatasetSummary};
use crate::datasets::errors;
use crate::uploads::da_repository::PersistentUploadRepository;

/// Rows asked for per page. DA's own default page size, and what web's page
/// requests.
pub const CATALOG_PAGE_SIZE: i64 = 200;

/// Pages walked before the list is reported as truncated.
///
/// Ten thousand datasets listed. Past that, paging further costs a round trip
/// per page to append rows nobody will scroll to, and saying the list is
/// partial is more useful than continuing.
pub const MAX_CATALOG_PAGES: usize = 50;

/// Largest dataset file this app will download.
///
/// The same ceiling the upload path enforces: a file that could not have been
/// uploaded here should not be streamed to disk here either, and the bound
/// keeps one download from filling a disk while the person watches a dialog.
pub const MAX_DOWNLOAD_BYTES: u64 = crate::uploads::service::MAX_UPLOAD_BYTES;

/// The account's ready datasets, in the order the workspace returned them.
///
/// Not sorted here: the destination sorts by whichever column the person
/// clicked, so a second ordering in this process would only be the one that
/// loses.
pub(crate) async fn list(
    repository: &dyn PersistentUploadRepository,
) -> Result<DatasetListResult, DatasetErrorCode> {
    let mut datasets = Vec::new();
    let mut uploads_enabled = true;
    let mut truncated = false;
    let mut offset = 0_i64;

    for page in 0..MAX_CATALOG_PAGES {
        let response = repository
            .list(CATALOG_PAGE_SIZE, offset)
            .await
            .map_err(|refusal| errors::from_refusal(&refusal))?;
        // Absent means enabled: a server that has not learned to report the
        // flag must not be read as one that turned uploads off.
        uploads_enabled = response.uploads_enabled.unwrap_or(true);
        let returned = response.items.len();
        datasets.extend(response.items.iter().filter_map(summary_view));

        // Advance by the page size asked for, not by the rows kept: filtering
        // out unready rows must not make the walk revisit them.
        offset += CATALOG_PAGE_SIZE;
        if returned == 0 || offset >= response.total {
            break;
        }
        if page + 1 == MAX_CATALOG_PAGES {
            truncated = true;
        }
    }

    Ok(DatasetListResult::new(datasets, uploads_enabled, truncated))
}

/// One dataset, with its files and their inferred schema.
pub(crate) async fn detail(
    repository: &dyn PersistentUploadRepository,
    dataset_id: &str,
) -> Result<DatasetDetail, DatasetErrorCode> {
    let response = repository
        .get(dataset_id)
        .await
        .map_err(|refusal| errors::from_refusal(&refusal))?;
    Ok(detail_view(&response))
}

/// Removes a dataset and everything DA holds for it.
pub(crate) async fn delete(
    repository: &dyn PersistentUploadRepository,
    dataset_id: &str,
) -> Result<(), DatasetErrorCode> {
    repository
        .delete(dataset_id)
        .await
        .map_err(|refusal| errors::from_refusal(&refusal))
}

/// Streams one dataset file to a path the person already chose.
///
/// The presigned URL is obtained, used, and dropped inside this process. It is
/// a bearer credential for tenant object storage: handing it to the webview
/// would put a read of the tenant's bucket inside the one process that renders
/// untrusted model output.
///
/// Returns the name DA reports for the file, so the caller can name the save
/// dialog's default without the webview having supplied it.
pub(crate) async fn download_name(
    repository: &dyn PersistentUploadRepository,
    dataset_id: &str,
    file_id: &str,
) -> Result<String, DatasetErrorCode> {
    let (_, filename) = repository
        .download_url(dataset_id, file_id)
        .await
        .map_err(|refusal| errors::from_refusal(&refusal))?;
    Ok(filename)
}

/// How long to wait for object storage to answer at all.
const DOWNLOAD_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// How long a transfer may stall before it is abandoned.
///
/// A read timeout rather than a total one: a 2 GiB file over a slow link is
/// slow, not stuck, and a wall-clock ceiling would kill exactly the transfers
/// that need the most patience.
const DOWNLOAD_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Fetches the file and writes it to `destination`.
pub(crate) async fn download_to(
    repository: &dyn PersistentUploadRepository,
    dataset_id: &str,
    file_id: &str,
    destination: &Path,
) -> Result<(), DatasetErrorCode> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    // Built here, with the same policy the upload path uses for its PUTs: no
    // redirects, because the presigned URL is the exact object and a 302 would
    // send a tenant-scoped request somewhere nobody signed for; and no retry,
    // because a retried range is a second read of a credential that may
    // already have expired.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(DOWNLOAD_CONNECT_TIMEOUT)
        .read_timeout(DOWNLOAD_READ_TIMEOUT)
        .build()
        .map_err(|_| DatasetErrorCode::Network)?;

    let (url, _) = repository
        .download_url(dataset_id, file_id)
        .await
        .map_err(|refusal| errors::from_refusal(&refusal))?;
    let response = client
        .get(url.expose())
        .send()
        .await
        .map_err(|_| DatasetErrorCode::Network)?;
    if !response.status().is_success() {
        return Err(DatasetErrorCode::ServerRefused);
    }

    let parent = destination.parent().ok_or(DatasetErrorCode::Storage)?;
    let temporary = tempfile::Builder::new()
        .prefix(".xfalcon-download-")
        .tempfile_in(parent)
        .map_err(|_| DatasetErrorCode::Storage)?
        .into_temp_path();
    let mut file = tokio::fs::File::create(&temporary)
        .await
        .map_err(|_| DatasetErrorCode::Storage)?;
    let mut written = 0_u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| DatasetErrorCode::Network)?;
        written = written.saturating_add(chunk.len() as u64);
        if written > MAX_DOWNLOAD_BYTES {
            // Abandoned rather than truncated: a partial file with the name
            // the person chose is worse than no file, because it looks like
            // the data.
            return Err(DatasetErrorCode::FileTooLarge);
        }
        if file.write_all(&chunk).await.is_err() {
            return Err(DatasetErrorCode::Storage);
        }
    }
    file.flush().await.map_err(|_| DatasetErrorCode::Storage)?;
    file.sync_all()
        .await
        .map_err(|_| DatasetErrorCode::Storage)?;
    drop(file);
    temporary
        .persist(destination)
        .map_err(|_| DatasetErrorCode::Storage)?;
    Ok(())
}

/// One list row, or nothing when the row is not a dataset a person can use.
fn summary_view(summary: &DatasetSummary) -> Option<DatasetView> {
    let status = parse_status(&summary.status);
    if status != DatasetStatus::Ready {
        return None;
    }
    Some(DatasetView {
        version: xfalcon_contracts::CONTRACT_VERSION,
        id: DatasetId::from(summary.dataset_id.as_str()),
        display_name: summary.display_name.clone(),
        status,
        created_at_ms: epoch_ms(&summary.created_at),
        updated_at_ms: epoch_ms(&summary.updated_at),
    })
}

fn detail_view(response: &DatasetDetailResponse) -> DatasetDetail {
    let dataset = DatasetView {
        version: xfalcon_contracts::CONTRACT_VERSION,
        id: DatasetId::from(response.dataset_id.as_str()),
        display_name: response.display_name.clone(),
        status: parse_status(&response.status),
        created_at_ms: epoch_ms(&response.created_at),
        updated_at_ms: response.updated_at.as_deref().map_or(0, epoch_ms),
    };
    let files = response
        .files
        .iter()
        .map(|file| DatasetFileView {
            file_id: file.file_id.clone(),
            table_name: file.table_name.clone(),
            row_count: file.row_count.and_then(|count| u64::try_from(count).ok()),
            column_count: file
                .column_count
                .and_then(|count| u32::try_from(count).ok()),
            size_bytes: file.size_bytes.and_then(|size| u64::try_from(size).ok()),
            original_filename: file.original_filename.clone(),
            columns: file
                .inferred_schema
                .as_ref()
                .map(|schema| {
                    // An ordered map, so the columns arrive in the order DA
                    // returned rather than in whatever order a hash produced.
                    schema
                        .iter()
                        .map(|(name, inferred_type)| DatasetColumnView {
                            name: name.clone(),
                            inferred_type: column_type(inferred_type),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            created_at_ms: file.created_at.as_deref().map_or(0, epoch_ms),
            updated_at_ms: file.updated_at.as_deref().map_or(0, epoch_ms),
        })
        .collect();
    DatasetDetail::new(dataset, files, response.uploads_enabled.unwrap_or(true))
}

/// A schema entry's type, as text.
///
/// DA types the value as free-form JSON. A string is the case in practice; a
/// number or an object is rendered as its JSON so the drawer shows what the
/// server actually said instead of an empty cell.
fn column_type(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// DA's status string. An unrecognised value is not `ready`, so it is treated
/// as one this app will not list — the safe direction, since listing an
/// unqueryable row is the failure that matters.
fn parse_status(status: &str) -> DatasetStatus {
    match status {
        "ready" => DatasetStatus::Ready,
        "uploading" => DatasetStatus::Uploading,
        _ => DatasetStatus::Failed,
    }
}

/// An ISO-8601 timestamp as epoch milliseconds.
///
/// DA sends both offset-bearing and naive forms; a naive one is read as UTC,
/// which is what DA documents them as. An unreadable timestamp becomes zero,
/// which sorts to the end of a newest-first list rather than to the top.
fn epoch_ms(timestamp: &str) -> i64 {
    use chrono::{DateTime, NaiveDateTime};

    if let Ok(parsed) = DateTime::parse_from_rfc3339(timestamp) {
        return parsed.timestamp_millis();
    }
    for format in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
    ] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(timestamp, format) {
            return naive.and_utc().timestamp_millis();
        }
    }
    0
}
