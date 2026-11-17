//! Where the macOS window buttons sit.
//!
//! The title bar is an overlay (`tauri.conf.json` `titleBarStyle`), so the close,
//! minimise and zoom buttons sit in the app's own 52px header row. The config's
//! `trafficLightPosition` asks tao to inset them, but tao re-applies that inset
//! only when its own view redraws, and the webview covering that view means it
//! almost never does: the buttons stayed at the system default, about 10px above
//! the row (Kyle, 2026-10-16). This places them ourselves, the same way tao does,
//! when the window is created and again whenever AppKit may have laid out its
//! title bar afresh: a resize (including leaving full screen), a focus change, or
//! a move between displays.
//!
//! A no-op off macOS, where the native title bar stays.

#[cfg(target_os = "macos")]
use tauri::WindowEvent;
use tauri::{Runtime, WebviewWindow};

/// The buttons' inset: 18pt from the left edge, and their top 19pt from the
/// window's top, which centres the 14pt buttons in the 52pt header row. Kept in
/// step with `trafficLightPosition` in `tauri.conf.json`.
#[cfg(target_os = "macos")]
const BUTTON_INSET: (f64, f64) = (18.0, 19.0);

/// Place the buttons now, and again after every event that can move them back.
pub fn keep_window_buttons_placed<R: Runtime>(window: &WebviewWindow<R>) {
    #[cfg(target_os = "macos")]
    {
        place_window_buttons(window);
        let handle = window.clone();
        window.on_window_event(move |event| {
            if matches!(
                event,
                WindowEvent::Resized(_)
                    | WindowEvent::Focused(_)
                    | WindowEvent::ScaleFactorChanged { .. }
            ) {
                place_window_buttons(&handle);
            }
        });
    }
    #[cfg(not(target_os = "macos"))]
    let _ = window;
}

#[cfg(target_os = "macos")]
fn place_window_buttons<R: Runtime>(window: &WebviewWindow<R>) {
    let handle = window.clone();
    if let Err(error) = window.run_on_main_thread(move || {
        let pointer = match handle.ns_window() {
            Ok(pointer) => pointer,
            Err(error) => {
                tracing::warn!(error = %error, "window buttons not placed: no NSWindow");
                return;
            }
        };
        // SAFETY: `ns_window` read this window's NSWindow just above, on the main
        // thread, and nothing runs between that read and this dereference. The
        // window is therefore still live, and the autoreleased pointer has not
        // drained. AppKit objects are only touched on the main thread, where this
        // closure runs.
        let ns_window = unsafe { &*pointer.cast::<objc2_app_kit::NSWindow>() };
        inset_buttons(ns_window, BUTTON_INSET);
    }) {
        tracing::warn!(error = %error, "window buttons not placed: main thread dispatch failed");
    }
}

/// tao's `inset_traffic_lights`: grow the title bar container so the buttons'
/// top lands `y` below the window's top, then space them from `x`.
#[cfg(target_os = "macos")]
fn inset_buttons(window: &objc2_app_kit::NSWindow, (x, y): (f64, f64)) {
    use objc2_app_kit::NSWindowButton;

    let (Some(close), Some(minimise), Some(zoom)) = (
        window.standardWindowButton(NSWindowButton::CloseButton),
        window.standardWindowButton(NSWindowButton::MiniaturizeButton),
        window.standardWindowButton(NSWindowButton::ZoomButton),
    ) else {
        tracing::warn!("window buttons not placed: a standard window button is missing");
        return;
    };
    // SAFETY: called on the main thread (see `place_window_buttons`), and the
    // buttons are owned by this live window, so their view chain is valid.
    let Some(container) = (unsafe { close.superview().and_then(|view| view.superview()) }) else {
        tracing::warn!("window buttons not placed: the title bar container is missing");
        return;
    };

    let close_frame = close.frame();
    let height = close_frame.size.height + y;
    let mut container_frame = container.frame();
    container_frame.size.height = height;
    container_frame.origin.y = window.frame().size.height - height;
    container.setFrame(container_frame);

    let spacing = minimise.frame().origin.x - close_frame.origin.x;
    for (index, button) in [close, minimise, zoom].iter().enumerate() {
        let mut origin = button.frame().origin;
        origin.x = x + index as f64 * spacing;
        button.setFrameOrigin(origin);
    }
}
