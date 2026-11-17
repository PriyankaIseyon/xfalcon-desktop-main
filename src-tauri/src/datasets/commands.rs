//! The eight commands the Datasets destination uses.
//!
//! Ids and names cross the boundary; paths, presigned URLs, and storage keys do
//! not. A file is picked here (`dataset_pick`), previewed by id
//! (`dataset_preview`), and uploaded by id (`dataset_upload_start`) — there is
//! deliberately no command that takes a path, and `dataset_download` takes no
//! destination, so `write_file` and `write_text_file` stay forbidden.
//!
//! Reading a file blocks. Every read runs on the blocking pool rather than on
//! the async runtime: a 50 MiB workbook parse on a runtime thread would stall
//! every other command, including the cancel the person reaches for when it
//! takes too long.

use std::path::PathBuf;
use std::sync::Arc;

use tauri::Manager as _;
use tauri_plugin_dialog::DialogExt as _;
use xfalcon_contracts::datasets::DatasetUploadStartResult;
use xfalcon_contracts::datasets::{
    DatasetColumnView, DatasetDetail, DatasetErrorCode, DatasetFormat, DatasetListResult,
    DatasetOperationError, DatasetPreview, DatasetPreviewUnavailable, DatasetRecipe,
    DatasetUploadRequest, StagedDatasetId, StagedDatasetView,
};
use xfalcon_contracts::export::DocumentExportResult;

use crate::datasets::preview::{self, Sample};
use crate::datasets::staging::Staging;
use crate::datasets::{catalog, errors, recipe};
use crate::export::{export_outcome, record_export};
use crate::uploads::da_repository::DaUploadRepository;
use crate::uploads::domain::DatasetIngest;
use crate::uploads::service::UploadService;
use crate::work_barrier::WorkBarrier;

/// Opens the native picker and stages what was chosen.
///
/// Staging, not uploading: the person has not seen the file yet.
#[tauri::command]
pub async fn dataset_pick(
    app: tauri::AppHandle,
) -> Result<StagedDatasetView, DatasetOperationError> {
    let staging = app.state::<Arc<Staging>>().inner().clone();
    let account_generation = staging.current_account_generation();
    let picked = app
        .dialog()
        .file()
        .add_filter("Data file", &ACCEPTED_EXTENSIONS)
        .blocking_pick_file()
        .ok_or_else(|| DatasetOperationError::new(DatasetErrorCode::NoFileChosen))?;
    let path = picked
        .into_path()
        .map_err(|_| DatasetOperationError::new(DatasetErrorCode::Unreadable))?;
    stage(&app, path, account_generation).await
}

/// A staged file's first rows, for the worksheet and header row asked for,
/// after the recipe.
#[tauri::command]
pub async fn dataset_preview(
    app: tauri::AppHandle,
    staged_id: StagedDatasetId,
    worksheet_index: Option<u32>,
    header_row_index: Option<u32>,
    recipe: Option<DatasetRecipe>,
) -> Result<DatasetPreview, DatasetOperationError> {
    let staging = app.state::<Arc<Staging>>().inner().clone();
    let source = staging
        .source(&staged_id)
        .map_err(DatasetOperationError::new)?;
    let worksheet_index = worksheet_index.unwrap_or(0);
    let recipe_available = source.size <= recipe::RECIPE_FULL_READ_MAX_BYTES;

    let sample = match staging.cached_sample(&staged_id, worksheet_index) {
        Some(sample) => Ok(sample),
        None => {
            let read = {
                let path = source.path.clone();
                let format = source.format;
                let size = source.size;
                tokio::task::spawn_blocking(move || {
                    preview::read_sample(&path, format, size, worksheet_index)
                })
                .await
                .map_err(|_| DatasetOperationError::new(DatasetErrorCode::Unreadable))?
            };
            if let Ok(sample) = &read {
                staging.cache_sample(&staged_id, sample.worksheet_index, sample.clone());
            }
            read
        }
    };

    let sample = match sample {
        Ok(sample) => sample,
        Err(unavailable) => {
            return Ok(unavailable_preview(staged_id, worksheet_index, unavailable))
        }
    };

    let recipe = recipe.unwrap_or_default();
    if !recipe.is_empty() && !recipe_available {
        return Err(DatasetOperationError::new(
            DatasetErrorCode::RecipeFileTooLarge,
        ));
    }
    build_preview(
        staged_id,
        &sample,
        header_row_index,
        &recipe,
        recipe_available,
    )
}

/// Releases a staged file the person did not upload.
#[tauri::command]
pub fn dataset_stage_discard(app: tauri::AppHandle, staged_id: StagedDatasetId) {
    app.state::<Arc<Staging>>().discard(&staged_id);
}

/// Uploads a staged file, applying its recipe first when it has one.
#[tauri::command]
pub async fn dataset_upload_start(
    app: tauri::AppHandle,
    request: DatasetUploadRequest,
) -> Result<DatasetUploadStartResult, DatasetOperationError> {
    let barrier = app.state::<Arc<WorkBarrier>>().inner().clone();
    let admission = barrier
        .admit_owned()
        .await
        .map_err(|_| DatasetOperationError::new(DatasetErrorCode::UpdatePending))?;
    let staging = app.state::<Arc<Staging>>().inner().clone();
    let service = app.state::<Arc<UploadService>>().inner().clone();
    let account_generation = service.current_account_generation();
    let source = staging
        .source(&request.staged_id)
        .map_err(DatasetOperationError::new)?;

    let recipe = request.recipe.clone().unwrap_or_default();
    let display_name = if request.display_name.trim().is_empty() {
        source.file_name.clone()
    } else {
        request.display_name.clone()
    };

    let (upload_path, ingest, lease) = if recipe.is_empty() {
        // No recipe: the picked file is sent as it is, and DA is told which
        // worksheet and header row to read. A header row of zero is DA's own
        // default, so it is omitted rather than asserted.
        let ingest = DatasetIngest {
            worksheet_index: request
                .worksheet_index
                .filter(|_| source.format.has_worksheets()),
            header_row_index: request.header_row_index.filter(|index| *index > 0),
        };
        (source.path.clone(), ingest, None)
    } else {
        // A recipe already chose the worksheet and consumed the header row, and
        // what it wrote is a CSV whose first row is the header — so neither
        // index is sent. Sending them would make DA skip a row of real data.
        let (written, lease) = materialize(&app, &source, &request, &recipe).await?;
        (written, DatasetIngest::default(), Some(lease))
    };

    let (upload_id, queue) = match service
        .enqueue_dataset(
            upload_path.clone(),
            &display_name,
            ingest,
            lease,
            account_generation,
        )
        .await
    {
        Ok(queue) => queue,
        Err(error) => {
            return Err(DatasetOperationError::new(errors::from_service(&error)));
        }
    };

    // The staged entry has done its job; the transfer holds the path it needs.
    staging.discard(&request.staged_id);

    let id = upload_id.as_str().to_owned();
    tokio::spawn(async move {
        let _admission = admission;
        if let Err(error) = service.start(&id).await {
            tracing::warn!(%error, upload = %id, "dataset upload start failed");
        }
    });
    Ok(DatasetUploadStartResult::new(upload_id, queue))
}

/// The account's ready datasets.
#[tauri::command]
pub async fn dataset_list(
    app: tauri::AppHandle,
) -> Result<DatasetListResult, DatasetOperationError> {
    let repository = repository(&app).await?;
    catalog::list(&repository)
        .await
        .map_err(DatasetOperationError::new)
}

/// One dataset, with its files and their inferred schema.
#[tauri::command]
pub async fn dataset_detail(
    app: tauri::AppHandle,
    dataset_id: String,
) -> Result<DatasetDetail, DatasetOperationError> {
    let repository = repository(&app).await?;
    catalog::detail(&repository, &dataset_id)
        .await
        .map_err(DatasetOperationError::new)
}

/// Removes a dataset.
#[tauri::command]
pub async fn dataset_delete(
    app: tauri::AppHandle,
    dataset_id: String,
) -> Result<(), DatasetOperationError> {
    let repository = repository(&app).await?;
    catalog::delete(&repository, &dataset_id)
        .await
        .map_err(DatasetOperationError::new)
}

/// Saves one of a dataset's files where the person chooses.
///
/// Two presigns rather than one: the first names the file so the save dialog
/// can suggest it, and the second is taken after the dialog closes so the URL
/// is fresh when the bytes move. A URL held across a dialog someone left open
/// would expire mid-transfer.
#[tauri::command]
pub async fn dataset_download(
    app: tauri::AppHandle,
    dataset_id: String,
    file_id: String,
) -> Result<DocumentExportResult, DatasetOperationError> {
    let (result, format) = match save_dataset_file(&app, &dataset_id, &file_id).await {
        Ok((saved, format)) => (Ok(saved), format),
        Err(error) => (Err(error), None),
    };
    record_export(&app, "dataset", format, export_outcome(&result));
    result
}

/// Returns the save result and, when a file was written, its known format.
async fn save_dataset_file(
    app: &tauri::AppHandle,
    dataset_id: &str,
    file_id: &str,
) -> Result<(DocumentExportResult, Option<&'static str>), DatasetOperationError> {
    let repository = repository(app).await?;
    let suggested = catalog::download_name(&repository, dataset_id, file_id)
        .await
        .map_err(DatasetOperationError::new)?;

    let (reply, chosen) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Save dataset file")
        .set_file_name(&suggested)
        .save_file(move |path| {
            let _ = reply.send(path);
        });
    let Some(path) = chosen
        .await
        .map_err(|_| DatasetOperationError::new(DatasetErrorCode::Storage))?
    else {
        // Dismissing the dialog is a decision, not a failure.
        return Ok((DocumentExportResult::cancelled(), None));
    };
    let destination = path
        .into_path()
        .map_err(|_| DatasetOperationError::new(DatasetErrorCode::Storage))?;

    catalog::download_to(&repository, dataset_id, file_id, &destination)
        .await
        .map_err(DatasetOperationError::new)?;
    let written = destination
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or(suggested);
    Ok((
        DocumentExportResult::written(written),
        known_format(&destination),
    ))
}

/// The file's extension when it names a format the app knows. The audit log
/// records the kind of file, never its name.
fn known_format(path: &std::path::Path) -> Option<&'static str> {
    const KNOWN: [&str; 8] = [
        "csv", "tsv", "xlsx", "parquet", "json", "jsonl", "txt", "pdf",
    ];
    let extension = path.extension()?.to_str()?;
    KNOWN
        .into_iter()
        .find(|known| extension.eq_ignore_ascii_case(known))
}

/// Stages a path, whether it came from the picker or from a drop.
pub(crate) async fn stage(
    app: &tauri::AppHandle,
    path: PathBuf,
    account_generation: u64,
) -> Result<StagedDatasetView, DatasetOperationError> {
    let staging = app.state::<Arc<Staging>>().inner().clone();
    tokio::task::spawn_blocking(move || staging.stage_for_generation(path, account_generation))
        .await
        .map_err(|_| DatasetOperationError::new(DatasetErrorCode::Unreadable))?
        .map_err(DatasetOperationError::new)
}

/// An authenticated repository for the catalog operations.
async fn repository(app: &tauri::AppHandle) -> Result<DaUploadRepository, DatasetOperationError> {
    let service = app.state::<Arc<UploadService>>().inner().clone();
    let (config, bearer) = service
        .authenticated()
        .await
        .map_err(|error| DatasetOperationError::new(errors::from_service(&error)))?;
    DaUploadRepository::new(config.da_base_url.as_str(), bearer)
        .map_err(|_| DatasetOperationError::new(DatasetErrorCode::NotConfigured))
}

/// Applies a recipe to the whole staged file and returns what was written.
async fn materialize(
    app: &tauri::AppHandle,
    source: &crate::datasets::staging::StagedSource,
    request: &DatasetUploadRequest,
    recipe: &DatasetRecipe,
) -> Result<(PathBuf, crate::security::temp_cleanup::TempLease), DatasetOperationError> {
    let directory = crate::datasets::staging::staging_directory(
        &app.path()
            .app_local_data_dir()
            .map_err(|_| DatasetOperationError::new(DatasetErrorCode::Storage))?,
    );
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|_| DatasetOperationError::new(DatasetErrorCode::Storage))?;

    let worksheet_index = request.worksheet_index.unwrap_or(0);
    let header_row_index = request.header_row_index.unwrap_or(0) as usize;
    let source = source.clone();
    let recipe = recipe.clone();
    let cleanup = app
        .state::<Arc<crate::security::temp_cleanup::TempCleanup>>()
        .inner()
        .clone();
    tokio::task::spawn_blocking(move || {
        recipe::materialize(
            &source,
            worksheet_index,
            header_row_index,
            &recipe,
            &directory,
            &cleanup,
        )
    })
    .await
    .map_err(|_| DatasetOperationError::new(DatasetErrorCode::Storage))?
}

/// A preview with nothing to show, and why.
fn unavailable_preview(
    staged_id: StagedDatasetId,
    worksheet_index: u32,
    unavailable: DatasetPreviewUnavailable,
) -> DatasetPreview {
    DatasetPreview {
        version: xfalcon_contracts::CONTRACT_VERSION,
        staged_id,
        worksheet_index,
        header_row_index: 0,
        suggested_header_row: 0,
        columns: Vec::new(),
        rows: Vec::new(),
        header_candidates: Vec::new(),
        sampled_rows: 0,
        truncated_rows: false,
        truncated_columns: false,
        truncated_cell: false,
        unavailable: Some(unavailable),
        // A file with no preview cannot be shaped either, whatever its size:
        // a recipe addresses columns this read did not produce.
        recipe_available: false,
    }
}

/// Resolves a sample into the preview the surface renders.
fn build_preview(
    staged_id: StagedDatasetId,
    sample: &Sample,
    header_row_index: Option<u32>,
    recipe: &DatasetRecipe,
    recipe_available: bool,
) -> Result<DatasetPreview, DatasetOperationError> {
    let suggested = preview::suggest_header_row(&sample.rows);
    let chosen = header_row_index.unwrap_or(suggested) as usize;
    let header = preview::header_at(sample, chosen);
    let (rows, more_rows) = preview::data_rows(sample, chosen);
    let sampled_rows = u32::try_from(rows.len()).unwrap_or(u32::MAX);

    let names = recipe::column_names(&header.columns);
    recipe::validate(recipe, &names)?;
    let shaped = recipe::apply(&names, &rows, recipe)?;

    // The recipe can rename, add, and drop columns, so the labels come from the
    // shaped table rather than from the file's own header.
    let mut columns: Vec<DatasetColumnView> = shaped
        .columns
        .iter()
        .map(|name| DatasetColumnView {
            name: name.clone(),
            inferred_type: String::new(),
        })
        .collect();
    preview::label_columns(&mut columns, &shaped.rows);

    Ok(DatasetPreview {
        version: xfalcon_contracts::CONTRACT_VERSION,
        staged_id,
        worksheet_index: sample.worksheet_index,
        header_row_index: u32::try_from(chosen).unwrap_or(0),
        suggested_header_row: suggested,
        columns,
        rows: shaped.rows,
        header_candidates: preview::header_candidates(sample),
        sampled_rows,
        truncated_rows: more_rows,
        truncated_columns: sample.truncated_columns,
        truncated_cell: sample.truncated_cell,
        unavailable: None,
        recipe_available,
    })
}

/// The format filter the picker offers. Named so the drop handler and the
/// picker cannot drift.
pub(crate) const ACCEPTED_EXTENSIONS: [&str; 2] = [
    DatasetFormat::Csv.extension(),
    DatasetFormat::Xlsx.extension(),
];
