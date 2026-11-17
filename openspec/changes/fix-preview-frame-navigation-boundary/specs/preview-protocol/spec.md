## MODIFIED Requirements

### Requirement: Per-response document policy and type

Every document response SHALL carry a Content-Security-Policy, a nosniff header,
and a Content-Type matching the registered kind.

The policy MUST deny by default and MUST allow the document's own inline
styling. It MUST deny every network avenue: no host sources anywhere, no
connections, no base URI, no form action.

Images MUST be restricted to inline data and blob sources; fonts to the origin's
own routes and inline data.

The font source MUST name the preview origin explicitly, in both platform
spellings, and MUST NOT be written as `'self'`. A preview frame is sandboxed
without `allow-same-origin`, so the document runs on an opaque origin that is
same-origin with nothing — `'self'` would match no URL at all, including the
font routes this handler serves, and an unstyled document would fall back to the
browser's serif default that the injected base stylesheet exists to prevent. A
policy string cannot branch on the platform, which is why both spellings are
named.

An HTML document MAY run its own inline scripts, because a dashboard's
interactivity is the point and previewed documents are self-contained by product
contract. An SVG document MUST be refused scripts outright — it carries image
semantics.

Frame ancestors MUST be pinned to the application's own origins, in both their
packaged and dev-server forms, so no other page can embed a preview.

An unknown or evicted id MUST answer with a not-found response that also carries
the nosniff header.

#### Scenario: SVG served as an image document

- **GIVEN** a registered SVG document
- **WHEN** its preview URL is fetched
- **THEN** the response Content-Type is the SVG image type and the document
  policy is present

#### Scenario: An SVG document runs no scripts

- **GIVEN** a registered SVG carrying a script element
- **WHEN** it renders
- **THEN** the script does not execute

#### Scenario: A preview cannot be embedded elsewhere

- **WHEN** the document policy is inspected
- **THEN** its frame-ancestors list names only the application's own origins

#### Scenario: The font source survives the sandbox's opaque origin

- **WHEN** any document policy is inspected
- **THEN** its font source names the preview origin in both platform spellings
- **AND** the policy contains no `'self'` source
