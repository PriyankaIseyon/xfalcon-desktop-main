## MODIFIED Requirements

### Requirement: A visualization request carries the output contract

The system MUST append the visualization contract block to every run's system prompt,
whatever the message and whichever skills are loaded. The block carries the saved-report
rules, and "save it" or "list my reports" needs them as much as a build request does, so
no gate decides it. The block is one fixed text, so the same request always gets the same
prompt.

Without that block, a request to build a dashboard or a report has nothing telling it what
those are in this product, and it is answered with SQL instead.

The contract MUST instruct the model to author self-contained HTML documents — every
script, style, and font inline, no external reference of any kind. The preview origin
serves each document under a policy that denies all network, so an external script tag is
not slow, it is absent, and the document renders blank. The contract MUST NOT name a CDN or
imply an external library is reachable.

#### Scenario: Build a dashboard

- **GIVEN** a message asking to build a dashboard
- **THEN** the run's system prompt contains the visualization contract

#### Scenario: A plain question carries it too

- **GIVEN** a message with no visualization intent
- **THEN** the run's system prompt still contains the visualization contract

#### Scenario: A loaded skill changes nothing

- **GIVEN** a loaded skill matching the BI pattern
- **THEN** the contract is present exactly as it is without one

#### Scenario: The guidance does not name a CDN

- **GIVEN** the visualization contract
- **THEN** it does not tell the model that an external script tag is available

## ADDED Requirements

### Requirement: The visualization contract teaches the assistant that DA stores and runs reports

The visualization contract text MUST require, for every dataset a dashboard shows, exactly
one inline `<script data-xf-dataset="ID">` carrier that assigns `window.XF_DATA["ID"]` an
object holding `rows` and `meta.lastRefresh`, and MUST require that every chart, KPI, and
table reads `window.XF_DATA["ID"].rows` at render time. It MUST require `<` inside the
carrier's JSON to be escaped as `\u003c`, and dataset ids to be limited to letters, digits,
`.`, `_`, and `-`.

It MUST state that DA stores, shares, refreshes, and deletes saved reports, that this app has
no save control for them, and that the assistant makes DA's tool calls: it MUST give the
save arguments (page, manifest marker, definitions, no payload rows), the single refresh
after a save, recall before rebuilding a named report, the delete confirmation rule, and
that the Reports page opens saved reports (not DA's inline viewer).

It MUST state the project-folder rules: save a report file into the project's `reports/`
folder with the project write tool when asked, bring a DA report into the folder by recall,
and put a local report on DA by reading it and calling DA's save tool.

The existing rule that the document has no external reference MUST remain. The contract MUST
NOT require a report-definition carrier script.

#### Scenario: A dashboard request

- **GIVEN** a turn that is a visualization request
- **THEN** the run's system prompt carries the dataset-carrier rule and the saved-report rules

#### Scenario: A plain question

- **GIVEN** a turn that is not a visualization request
- **THEN** the system prompt still carries the saved-report rules, so "save it" and "list my
  reports" work

#### Scenario: The self-contained rule survives

- **GIVEN** the same prompt
- **THEN** it still forbids `<script src>`, stylesheets, remote fonts, and remote images
