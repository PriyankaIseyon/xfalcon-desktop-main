//! Windows WebView2 `PrintToPdf` — write the mounted print sheet without
//! blocking the renderer on a print preview (spec: lean-report-export).
//!
//! `window.print()` on WebView2 is synchronous. Chromium fires `beforeprint`
//! and then holds the JS thread until its preview UI is dismissed. That
//! preview can fail to appear in a Tauri window, which left Close and Cancel
//! as no-ops for the life of the process. `PrintToPdf` uses the same print
//! engine and `@media print` rules, but it is a host-side async COM call, so
//! the save dialog and the export dialog stay cancellable.

use std::path::Path;
use std::time::Duration;

use tauri::webview::PlatformWebview;
use tokio::sync::oneshot;
use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2PrintSettings, ICoreWebView2_7};
use webview2_com::PrintToPdfCompletedHandler;
use windows::core::{Interface, HSTRING};

use super::{create_export_temp, log_write_failure, persist_export, DocumentExportError};

const PRINT_TIMEOUT: Duration = Duration::from_secs(60);

pub async fn write_pdf(
    window: &tauri::WebviewWindow,
    destination: &Path,
) -> Result<(), DocumentExportError> {
    // WebView2 overwrites its target before the async job is known to have
    // succeeded. Print beside the chosen file and replace it only after the
    // completion callback, so a failed job cannot damage an existing PDF.
    let temporary = create_export_temp(destination, "pdf")?.into_temp_path();
    write_pdf_to_path(window, &temporary).await?;
    std::fs::OpenOptions::new()
        .write(true)
        .open(&temporary)
        .and_then(|file| file.sync_all())
        .map_err(log_write_failure)?;
    persist_export(temporary, destination)
}

async fn write_pdf_to_path(
    window: &tauri::WebviewWindow,
    temporary: &Path,
) -> Result<(), DocumentExportError> {
    let (tx, rx) = oneshot::channel();
    let path = temporary.to_path_buf();
    window
        .with_webview(move |webview| start(&webview, &path, tx))
        .map_err(|_| DocumentExportError::PrintFailed)?;

    match tokio::time::timeout(PRINT_TIMEOUT, rx).await {
        Ok(Ok(Ok(()))) => Ok(()),
        Ok(Ok(Err(error))) => Err(error),
        Ok(Err(_)) => Err(DocumentExportError::PrintFailed),
        Err(_) => Err(DocumentExportError::PrintTimedOut),
    }
}

fn start(
    webview: &PlatformWebview,
    destination: &Path,
    tx: oneshot::Sender<Result<(), DocumentExportError>>,
) {
    let Some(path) = destination.to_str() else {
        let _ = tx.send(Err(DocumentExportError::UnusableDestination));
        return;
    };

    let controller = webview.controller();
    // SAFETY: `with_webview` runs `start` on the main thread that owns the WebView2
    // objects. `controller` is an owned COM reference cloned by
    // `PlatformWebview::controller`, so the call uses a reference that is alive.
    let core = match unsafe { controller.CoreWebView2() } {
        Ok(core) => core,
        Err(_) => {
            let _ = tx.send(Err(DocumentExportError::PrintFailed));
            return;
        }
    };
    let printer = match core.cast::<ICoreWebView2_7>() {
        Ok(printer) => printer,
        Err(_) => {
            let _ = tx.send(Err(DocumentExportError::PrintFailed));
            return;
        }
    };

    let cleanup_path = destination.to_path_buf();
    let path = HSTRING::from(path);
    let handler = PrintToPdfCompletedHandler::create(Box::new(move |status, success| {
        let outcome = if status.is_ok() && success {
            Ok(())
        } else {
            Err(DocumentExportError::PrintFailed)
        };
        // A timed-out receiver has gone away. WebView2 cannot cancel this
        // operation, so remove any late file when its callback finally runs.
        if tx.send(outcome).is_err() {
            let _ = std::fs::remove_file(&cleanup_path);
        }
        Ok(())
    }));

    // COM AddRefs the handler. If this call fails, `handler` drops and the
    // oneshot disconnects, which `write_pdf` already treats as PrintFailed.
    // SAFETY: `printer` was cast from this webview's CoreWebView2 on the same main
    // thread. `path` is an HSTRING that lives until this function returns, which
    // covers the call. `None` passes no print settings. `handler` is passed by
    // reference, and COM AddRefs it (see the comment above), so the callback can
    // outlive this frame.
    let _ = unsafe { printer.PrintToPdf(&path, None::<&ICoreWebView2PrintSettings>, &handler) };
}
