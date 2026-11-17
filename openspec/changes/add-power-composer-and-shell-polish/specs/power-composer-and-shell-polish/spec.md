# power-composer-and-shell-polish — Spec Delta

## ADDED Requirements

### Requirement: Resource tagging with `@` mentions and slash commands in composer

The composer MUST provide fuzzy auto-completion when the user types `@`, allowing immediate referencing of bound local files, MCP servers, and saved reports as rich context chips.

The composer MUST display an interactive action list when typing `/`, enabling context clearance, skill attach, project navigation, and MCP settings. Model picking is out of scope.

Imported skills MUST appear in the `@` mention list as `@skill:<name>`. Typing `/skill` with imported skills MUST open that skill list rather than only jumping to Settings. Selecting a skill SHALL insert `@skill:<name>` into the draft so the run can fetch it by name. With an empty catalog, `/skill` SHALL open Settings so the reader can import.

#### Scenario: Referencing a local data file in prompt

- **GIVEN** a user typing in the composer
- **WHEN** the user types `@sales`
- **THEN** a fuzzy dropdown suggests `sales_data.csv`, and selecting it attaches a context pill to the prompt

#### Scenario: Mentioning a skill

- **GIVEN** an imported enabled skill named `financial-analysis`
- **WHEN** the reader types `@skill` in the composer
- **THEN** the mention list offers `@skill:financial-analysis`
- **AND** selecting it inserts that token into the draft

#### Scenario: Slash skill with a catalog

- **GIVEN** at least one imported skill
- **WHEN** the reader submits `/skill`
- **THEN** the composer opens the skill mention list
- **AND** Settings is not opened

#### Scenario: Slash skill with an empty catalog

- **GIVEN** no imported skills
- **WHEN** the reader submits `/skill`
- **THEN** Settings opens on the skills section

### Requirement: Focus mode

Native backdrop translucency is deferred to `update-product-wide-premium-ux`
task 8.3. Its opaque fallback and platform behavior must be decided and proved
there as one contract.

The interface MUST provide a Focus/Zen mode shortcut that collapses all side panels for distraction-free analysis and reading.

#### Scenario: Toggling Zen / Focus mode

- **GIVEN** a workspace with active sidebar and canvas open
- **WHEN** the user triggers the Focus Mode shortcut (`Cmd+Shift+F` / `Ctrl+Shift+F`)
- **THEN** the sidebars smoothly collapse, centering the chat surface with full focus on the content
