## ADDED Requirements

### Requirement: A report card shows the image DA stored for it

The system MUST show a report card's stored thumbnail when DA lists one, and MUST NOT open the
report to draw that card. It MUST accept only a PNG or WebP image of at most 200 KiB whose
bytes match the declared type. It MUST draw the report itself only when DA holds no image. It
MUST ask DA at most once per image version while the image is held, and MUST release every
held image on sign-out.

#### Scenario: DA holds an image

- **GIVEN** a report whose listing names a thumbnail version
- **WHEN** its card comes on screen
- **THEN** the card shows the stored image and the report is not opened

#### Scenario: DA holds no image

- **GIVEN** a report whose listing names no thumbnail, or whose image DA no longer holds
- **WHEN** its card comes on screen
- **THEN** the card draws the report itself

#### Scenario: The image cannot be read

- **GIVEN** a report whose listing names a thumbnail version
- **WHEN** DA cannot be reached, or answers with something that is not that image
- **THEN** the card says the preview is unavailable and the report is not opened

### Requirement: A tool for a server's own view is not offered to the model

The system MUST NOT offer the model an MCP tool whose `_meta.ui.visibility` excludes `model`.

#### Scenario: DA publishes its thumbnail upload tool

- **GIVEN** DA lists `ida_upload_dashboard_thumbnail` with visibility `["app"]`
- **WHEN** desktop builds the run's tool list
- **THEN** that tool is not in it

### Requirement: An owner can make a dashboard's thumbnail

The system MUST let only a dashboard's owner make its thumbnail, from the report's own first
page, and MUST store it only after checking that it is a PNG within 200 KiB. It MUST make the
image once, in the background, when the owner opens a dashboard that has none, and MUST offer
the owner an action to make it again and a way to make every missing one. A failed capture
MUST leave the report and any image it had unchanged.

#### Scenario: An owner opens a dashboard with no image

- **GIVEN** a dashboard the reader owns, with no stored image
- **WHEN** the reader opens it and its page loads
- **THEN** an image is made and stored, and its card shows it

#### Scenario: A reader who is not the owner

- **GIVEN** a dashboard shared with the reader
- **THEN** no preview action is offered, and opening it makes no image
