# Change: Staging desktop update delivery

## Why

The private GitHub draft endpoint cannot serve ordinary desktop clients. The
current Windows installer exits during download/install, preventing a deferred
restart experience.

## What Changes

- Deliver staging updates through CloudFront with a private S3 origin.
- Upload immutable signed candidates using GitHub OIDC; promote a selected
  version to the staging channel only after asset validation.
- Check the actual installed version at startup and every four hours.
- Show an app-wide update notice; automatically download and verify without blocking work.
- Persist verified packages and apply only on explicit restart while idle.

## Impact

Desktop update contracts, Rust service, frontend notice/settings, release
workflows, staging AWS infrastructure, and operational documentation.

## Authorization

The user approved implementing the researched staging updater plan in this
conversation on 2026-09-07. Existing unrelated working-tree changes are retained.
