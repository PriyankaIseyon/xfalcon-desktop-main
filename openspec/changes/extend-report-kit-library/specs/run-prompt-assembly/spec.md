## ADDED Requirements

### Requirement: The visualization contract names every kind the Reports page lists

The visualization contract MUST tell the model that the Reports page lists dashboards,
workbooks, and presentations, that a workbook or presentation is a file the reader downloads
from it, and that it shows a report the reader hid only when they ask for hidden reports. It
MUST NOT tell the model that a kind is missing from the Reports page, because the model would
repeat that to a reader who can see it there.

#### Scenario: The model lists reports

- **WHEN** the model is asked what reports exist and the server also returns a workbook
- **THEN** its prompt tells it to say which kind each is, and does not say the Reports page hides workbooks
