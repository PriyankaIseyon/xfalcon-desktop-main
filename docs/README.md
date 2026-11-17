# xFalcon Desktop documentation

Source version 0.5.51 · reviewed 2026-11-04

This index describes the 0.5.51 source baseline. It does not establish release approval or general availability.

Everything in this folder is Markdown. The only subdirectory is `releases/`, which
holds one user-facing release note per version. There are no specifications here
— the living specification is `openspec/specs/`, and in-flight proposals are
`openspec/changes/`. These documents explain, instruct, and record; they never
define requirements.

## Find what you need

The four columns follow [Diátaxis](https://diataxis.fr/): learning, doing,
looking up, understanding. Pick the column that matches what you are trying to
do, not the topic you are thinking about.

| I want to…                                                                  | Read                                                                          |
| --------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| **Learn** — get the app running and send my first turn                      | [getting-started.md](getting-started.md)                                      |
| **Do** — build, sign, and ship a release                                    | [operations.md](operations.md) · [v0.5.51 release notes](releases/v0.5.51.md) |
| **Do** — make a change and get it merged                                    | [contributing.md](contributing.md)                                            |
| **Do** — fix something that is broken                                       | [troubleshooting.md](troubleshooting.md)                                      |
| **Do** — sign and notarise a release, and plan Windows signing              | [code-signing.md](code-signing.md)                                            |
| **Look up** — a Tauri command, an event, a contract type                    | [ipc-reference.md](ipc-reference.md)                                          |
| **Look up** — a tool schema, a skill field, a hard limit                    | [agent-runtime-reference.md](agent-runtime-reference.md)                      |
| **Look up** — an environment variable, a file on disk                       | [configuration.md](configuration.md)                                          |
| **Look up** — what the local audit log records and how to read it           | [audit-log.md](audit-log.md)                                                  |
| **Look up** — what data the app stores, for how long, and how it is removed | [data-lifecycle.md](data-lifecycle.md)                                        |
| **Look up** — a term I do not recognise                                     | [glossary.md](glossary.md)                                                    |
| **Look up** — a shipped font and its licence                                | [fonts.md](fonts.md)                                                          |
| **Understand** — how the system is put together                             | [architecture.md](architecture.md)                                            |
| **Understand** — what happens during a chat turn                            | [runtime-flows.md](runtime-flows.md)                                          |
| **Understand** — why it is built this way                                   | [decisions.md](decisions.md)                                                  |
| **Understand** — the security model and what enforces it                    | [security.md](security.md)                                                    |
| **Understand** — which SOC 2 criteria map to which desktop controls         | [soc2-control-mapping.md](soc2-control-mapping.md)                            |
| **Understand** — what is verified, what is planned, and what is still open  | [assurance-plan.md](assurance-plan.md)                                        |
| **Understand** — what is actually finished                                  | [status.md](status.md)                                                        |

## Audience map

| You are                                  | Start here                                         | Then                                                                                           |
| ---------------------------------------- | -------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| New engineer on the team                 | [getting-started.md](getting-started.md)           | [architecture.md](architecture.md), [contributing.md](contributing.md)                         |
| Engineer changing the agent or tool loop | [runtime-flows.md](runtime-flows.md)               | [agent-runtime-reference.md](agent-runtime-reference.md), [decisions.md](decisions.md)         |
| Engineer changing the UI                 | [architecture.md](architecture.md) § renderer      | [ipc-reference.md](ipc-reference.md), `openspec/specs/`                                        |
| Security reviewer                        | [security.md](security.md)                         | [threat-model.md](threat-model.md), [decisions.md](decisions.md)                               |
| Auditor or compliance owner              | [compliance.md](compliance.md)                     | [threat-model.md](threat-model.md), [security.md](security.md)                                 |
| Release engineer                         | [operations.md](operations.md)                     | [code-signing.md](code-signing.md), [configuration.md](configuration.md), [fonts.md](fonts.md) |
| Support engineer                         | [troubleshooting.md](troubleshooting.md)           | [audit-log.md](audit-log.md), [configuration.md](configuration.md) § on-disk layout            |
| Compliance owner                         | [soc2-control-mapping.md](soc2-control-mapping.md) | [assurance-plan.md](assurance-plan.md), [data-lifecycle.md](data-lifecycle.md)                 |
| Product or leadership                    | [status.md](status.md)                             | [architecture.md](architecture.md) § what this product is                                      |
| An AI coding agent                       | `AGENTS.md` at the repo root                       | [contributing.md](contributing.md), [decisions.md](decisions.md)                               |

## The complete set

**Learning** — [getting-started.md](getting-started.md)

**Doing** — [code-signing.md](code-signing.md) · [contributing.md](contributing.md) · [operations.md](operations.md) · [troubleshooting.md](troubleshooting.md)

**Looking up** — [audit-log.md](audit-log.md) · [data-lifecycle.md](data-lifecycle.md) · [ipc-reference.md](ipc-reference.md) · [agent-runtime-reference.md](agent-runtime-reference.md) · [configuration.md](configuration.md) · [threat-model.md](threat-model.md) · [compliance.md](compliance.md) · [fonts.md](fonts.md) · [glossary.md](glossary.md)

**Understanding** — [architecture.md](architecture.md) · [assurance-plan.md](assurance-plan.md) · [runtime-flows.md](runtime-flows.md) · [decisions.md](decisions.md) · [security.md](security.md) · [soc2-control-mapping.md](soc2-control-mapping.md) · [status.md](status.md)

## Documents outside the four columns

These files record a point in time, mirror another repository, or describe work
that is not finished. The 2026-10-15 review refreshed `desktop-updates-staging.md`;
the other entries remain dated records and must be checked against their source
before relying on them.

| File                                                                       | What it is                                                                                                                                                                                                                                 |
| -------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| [xfalcon-platform.md](xfalcon-platform.md)                                 | A multi-repository technical reference for the whole platform. It was written against the working trees of 2026-09-17. Its desktop sections (§ 33–40) were re-read for 0.5.36; its other sections describe other repositories and were not |
| [desktop-updates-staging.md](desktop-updates-staging.md)                   | The AWS delivery path for the staging update channel (CloudFront, the publisher role, the object layout). [operations.md](operations.md) covers the release workflow                                                                       |
| [refreshable-reports-architecture.md](refreshable-reports-architecture.md) | A proposed design, maintained as an identical file in the `data-analyst` repository. It states it is not deployed behaviour                                                                                                                |
| [dashboard-feature-roadmap.md](dashboard-feature-roadmap.md)               | A roadmap (version 0.1, 2026-08-25). It is a plan, not a description of the code                                                                                                                                                           |
| [chat-streaming-presentation.md](chat-streaming-presentation.md)           | A research and implementation review of streaming defects (2026-09-16)                                                                                                                                                                     |
| [staging-main-audit.md](staging-main-audit.md)                             | An audit of the staging-to-main range as it stood on 2026-09-16                                                                                                                                                                            |
| [project-deletion-audit.md](project-deletion-audit.md)                     | A pull-request review of project availability and conversation cleanup                                                                                                                                                                     |
| `releases/`                                                                | Release notes from `v0.5.8.md` and `v0.5.14.md` through `v0.5.51.md`; `v0.5.9.md` to `v0.5.13.md` are absent. The release workflow reads `releases/v<version>.md` ([operations.md](operations.md))                                         |

## House rules for these documents

1. **No specifications.** Requirements live in `openspec/specs/`. If a sentence
   here reads like a `SHALL`, it belongs there instead.
2. **No plans.** In-flight work lives in `openspec/changes/`. Historical plans
   are in git history, not in this folder. (`dashboard-feature-roadmap.md` and
   `refreshable-reports-architecture.md` predate this rule and break it.)
3. **Claims carry evidence.** A statement about behaviour names the file that
   implements it or the test that proves it. A statement that cannot do that is
   marked as unverified.
4. **Stale is broken.** A document that contradicts the code is a defect of the
   same severity as a failing test. Fix it in the change that caused the drift.
5. **Stable identifiers.** `TM-###` threat identifiers and `CM-###` control
   identifiers are cited from source comments. Renumbering them breaks those
   citations, so entries are retired in place, never reused or renumbered.
6. **One reviewed date per file.** The header line carries the version and the
   date the file was last read against the code.
