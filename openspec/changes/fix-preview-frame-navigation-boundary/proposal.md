# Change: Let a preview frame load on WebKit

## Why

An HTML or SVG artifact renders as a blank white panel in the macOS build. The
same document, saved and opened in a browser, renders correctly. Nothing is
logged, and the card reports no failure.

The cause is not in the preview handler. It is in the navigation boundary.
`allows_navigation` (`src-tauri/src/security/capabilities.rs`) admits exactly
the app origin, and it was written for one job: stop the main document
navigating away from the app. It is wired to Tauri's `on_navigation`, and the
two engines disagree about which loads reach that hook.

- WebView2 raises `NavigationStarting` for the **top-level document only**.
  An iframe raises the separate `FrameNavigationStarting`, which wry does not
  subscribe to. On Windows a preview frame is never policed, so it loads.
- WKWebView asks `decidePolicyForNavigationAction:` for **every frame**, and
  wry forwards all of them with no main-frame test
  (`wry-0.55.1/src/wkwebview/navigation.rs:76`). On macOS the preview frame IS
  policed by the same function — which answers `Cancel`, because
  `xf-preview://localhost/<id>` is not the app origin.

A cancelled frame navigation is silent. The empty document still fires `load`,
so `onLoad` runs, the frame fades in, and the reader gets a blank panel where
the report should be. Every preview surface is affected: the chat card, the
canvas panel, and the report viewer.

The second defect is in the same path and would surface the moment the first is
fixed. Every preview frame is sandboxed without `allow-same-origin`, so the
document runs on an **opaque** origin — the handler already says so, in the
comment explaining why the font routes need `Access-Control-Allow-Origin`. An
opaque origin is same-origin with nothing, so the document policy's
`font-src 'self'` names no URL at all, including the `/fonts/` routes that same
handler serves. A document that declares no typography would then fall back to
the browser's serif — the exact defect the injected base stylesheet exists to
prevent.

Both defects are invisible to the local gate, which runs one host. `#[cfg]`
arms for the other platform are never compiled here, let alone run.

## What Changes

- The navigation boundary admits the preview origin, in this platform's
  spelling only, and refuses every neighbour of it: a foreign host, a port,
  embedded credentials, a neighbouring scheme, and the spelling this build
  cannot mint.
- The document policy names the preview origin as its font source instead of
  `'self'`, in both platform spellings, because a policy string cannot branch
  on the platform.
- Platform-conditional origin matching moves from `#[cfg]` to `cfg!`, so both
  arms type-check on whichever host the gate happens to run. The same move is
  applied to the tests, which exercise both spellings on every host.

## Impact

- Affected specs: `webview-capability-lockdown`, `preview-protocol`
- Affected code: `src-tauri/src/security/capabilities.rs`,
  `src-tauri/src/preview/protocol.rs`, `src-tauri/src/preview/mod.rs`
- No behaviour changes on Windows: the boundary was never consulted for a
  subframe there, and the font source resolves to the same origin `'self'`
  already meant.
