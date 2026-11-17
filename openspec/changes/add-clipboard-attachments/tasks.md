## 1. Implementation and validation

- [x] 1.1 Paste FileList and file items through the composer while preserving text/HTML paste and busy-state behavior.
- [x] 1.2 Queue clipboard bytes and native Windows/macOS file references through the existing upload flow.
- [x] 1.3 Validate size/type/content and temporary-source lifecycle in native code.
- [x] 1.4 Test the real composer and chat-action consumers plus native upload boundaries.
- [x] 1.5 Run frontend checks and Windows native checks, rebuild and relaunch locally.
- [ ] 1.6 Verify native clipboard behavior interactively on Windows and macOS.

## Evidence

- Frontend: `npm test` passed (128 Vitest files, 1,749 tests, plus release/script checks); typecheck, build, targeted ESLint and formatting passed.
- Windows: `cargo test --manifest-path src-tauri\Cargo.toml uploads:: --lib` passed 74 tests; `security::capabilities --lib` passed 3 tests.
- macOS: actual clipboard module and native source-opening helper type-checked for `aarch64-apple-darwin` with matching direct dependency versions; full app build and runtime paste were not available on this host.
- Interactive Windows automation was unavailable: the computer-use runtime rejected this WSL workspace URI before connecting.
- Windows custom-protocol build passed; rebuilt executable launched from `D:\work\xfalcon\xfalcon-desktop\target\debug\xfalcon-desktop.exe` and process reported responsive.
