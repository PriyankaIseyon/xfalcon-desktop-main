## ADDED Requirements

### Requirement: The navigation boundary governs every frame

The system MUST decide, for each document the webview is asked to load, whether
that document may load. The decision SHALL admit the application's own origin in
the spelling the running platform produces, and the preview origin in the
spelling the running platform produces, and nothing else.

The boundary MUST be written as if it governs subframes, because on WebKit it
does. WebView2 consults it for the top-level document only; WKWebView consults
it for every frame. A boundary naming only the application origin therefore
cancels the preview frame on macOS, and a cancelled frame navigation reports
nothing: the empty document still fires its load event, so the surface settles
blank with no error to find.

An origin that differs from an admitted one by host, port, scheme, or embedded
credentials MUST be refused. The platform spelling this build cannot mint MUST
be refused, because `.localhost` resolves to loopback and the unused spelling
would otherwise name whatever local server answers on it.

Platform-conditional matching MUST be written so that every platform's arm
type-checks on every host. Attribute-gated arms MUST NOT be used for it: the
local gate runs a single host, so an arm for the other platform is never
compiled there and a defect in it stays invisible until release.

#### Scenario: A preview frame loads

- **GIVEN** a registered preview document
- **WHEN** a preview frame is asked to load it from the preview origin
- **THEN** the navigation is admitted, on every platform

#### Scenario: A near neighbour of an admitted origin is refused

- **WHEN** a URL differs from an admitted origin by host, by port, by scheme, or
  by carrying credentials
- **THEN** the navigation is refused

#### Scenario: The inactive platform spelling is refused

- **WHEN** a URL uses the origin spelling this build's engine never produces
- **THEN** the navigation is refused

#### Scenario: Both platform arms are exercised on one host

- **WHEN** the navigation tests run on either host
- **THEN** both platform spellings are compiled and asserted, with the active
  one admitted and the inactive one refused
