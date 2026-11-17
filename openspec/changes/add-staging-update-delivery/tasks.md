## Implementation and verification

- [x] Private CloudFront/OAC and scoped GitHub OIDC resources created; account and staging-only scope verified. Viewer delivery enabled after user approval; S3 and candidate paths return HTTP 403 without authorization.
- [x] Candidate publishing, cryptographic verification, and conditional promotion pass 15 Node tests; signed v0.5.4 candidate uploaded privately.
- [x] Native staging, cache recovery, verification, and work guards pass 11 native tests.
- [x] App-wide notice, centered Settings notice, and sidebar count badge pass component and real-browser checks.
- [x] CloudFront serves the signed 0.5.4 artifacts and manifest; channel requires revalidation. GitHub run 34137963305 passed after initial edge propagation.
- [x] Frontend build, full unit suites, Windows native tests, and Linux/Windows clippy pass.
- [x] Windows application rebuilt and relaunched with the staging overlay; running application reports xFalcon, responding, version 0.5.4.
- [x] GitHub staging OIDC release uploads and promotion executed successfully on the pushed workflow.
- [ ] Packaged Windows and macOS upgrade between different versions verified.

- [x] Startup automatically downloads a newer version with no click; real-browser test verifies update_check then update_install and no restart call. Full frontend suite passes 1,640 tests.
