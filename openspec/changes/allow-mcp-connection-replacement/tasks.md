## Implementation and verification

- [x] Persist bundled MCP removal and restoration, preserve account login, and exclude removed tools from runtime sets after restart.
- [x] Support optional OAuth client details for add, edit and retry without leaking into registry payloads or other servers.
- [x] Verify required workspace login configuration in both packaged platforms against build inputs.
- [x] Pass focused native, renderer, contract and release-script regression tests.
- [x] Pass frontend gate and relevant browser checks; rebuild and relaunch the Windows app.
- [x] Audit the complete diff before commit, push and release dispatch.

Verification on 2026-09-16: frontend gate passed (1,733 tests), production build passed, release scripts passed (44 tests), Settings and OAuth browser checks passed (8 tests). Linux workspace tests passed (1,741 tests; the existing large-tree benchmark remains ignored), strict Clippy and formatting passed. Windows MCP tests passed (249 tests). Independent review covered UI, packaging, IPC permissions and OAuth resource binding; all findings were resolved and regression-tested. The release workflow will additionally verify the actual macOS and Windows bundle contents.

The Windows app was rebuilt with the production frontend and staging updater configuration, then relaunched. Startup reported version 0.5.14 and resolved the bundled tenant configuration; the xFalcon window was verified running.
