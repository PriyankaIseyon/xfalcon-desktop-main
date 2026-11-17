# DA OpenAPI contract

This directory contains the complete, immutable OpenAPI 3.1 snapshot used to
generate xFalcon's crate-private DA DTOs.

- Source: `https://iseyon.mcp-staging.iseyon.com/openapi.json`
- OpenAPI: `3.1.0`
- API title: `IDA MCP Server`
- API version: `0.1.10`
- SHA-256: `048be51a7eab7f651f9f801ed30017f88daf9b33542228e501b6d85fa5ba7ac4`
- DA source revision: `beb7ebcfa2ff43ddf4e0ebd657c460755abae940`
- The staging deploy matches no single DA commit. Its report-kit paths and `Kit*` schemas equal DA main from `983f44c` through `fe71674`; the revision above is the `v0.1.10` release commit.
- Generator: `oas3-gen 0.26.2`
- Post-processing: `rustfmt --edition 2021`

Regenerate from the repository root with an explicit path to the pinned
generator:

```powershell
pwsh ./scripts/sync-da-contract.ps1 -GeneratorPath <path-to-oas3-gen-0.26.2>
```

The script validates the exact source, metadata, generator version, and SHA-256
before replacing any committed artifact. To advance the immutable pin, update
`PinnedSha256` in the script in the same reviewed change, then invoke the script
with both `-AdvancePin` and an explicit `-ExpectedSha256`.

The generated Rust output is intentionally crate-private and has all `Debug`
derives removed so secret-bearing DA models cannot be formatted into logs.
