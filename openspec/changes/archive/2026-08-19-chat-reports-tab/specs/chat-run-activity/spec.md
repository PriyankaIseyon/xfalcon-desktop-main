# chat-run-activity — Spec Delta

## ADDED Requirements

### Requirement: A dashboard the model authors MUST render with no network

The system MUST instruct the model to author self-contained HTML documents:
every script, style, and font inline, and no external reference of any kind.
The preview origin serves each document under a policy that denies all
network — no host sources, no connections — so an external `<script src>` is
not slow, it is absent, and the document renders blank.

The visualization guidance currently offers the opposite: it names jsDelivr
and unpkg and tells the model an external library means a mandatory
`<script src>` in `<head>`. That guidance MUST be replaced rather than
qualified. A hint the runtime cannot honour is worse than no hint, because
the model follows it confidently.

Allowing those hosts in the document policy is the rejected alternative. It
would restore parity with xfalcon-web at the cost of opening a network
egress channel out of model-authored content, contradicting the deny-all
posture the preview origin exists to hold — and it would make an offline
desktop render worse than an online one, which is backwards for a
local-first product.

Reports are what make this urgent. A blank chat preview is a bad turn the
user can retry. A blank _saved_ report is a permanent artifact the user comes
back to, and it was blank at the moment it was saved.

#### Scenario: A dashboard renders offline

- **GIVEN** a request for a chart or dashboard, with no network available
- **THEN** the document the model returns renders, because it carries
  everything it draws with

#### Scenario: The guidance does not name a CDN

- **GIVEN** the visualization guidance a BI turn carries
- **THEN** it does not tell the model that an external script tag is
  available, because the runtime will not load one

### Requirement: A document that cannot render is not saved silently

The system MUST warn the user before storing a report whose document
references an external script or stylesheet. That document will render blank
from the preview origin, and storing it without a word produces a saved
report that is broken on the day it is created.

The warning MUST be part of the naming step, where the user can still
decline, rather than a notice afterwards about something already stored.

Detection MUST NOT block the save. It is a heuristic over markup, and a
false positive that refuses a working dashboard would be worse than the
warning it was trying to give.

#### Scenario: A dashboard with an external script

- **GIVEN** a document with a `<script src>` pointing at another host
- **WHEN** the user goes to save it as a report
- **THEN** the naming step says it will not render, and the user can save it
  anyway or cancel

#### Scenario: A self-contained dashboard

- **GIVEN** a document with everything inline
- **WHEN** the user goes to save it
- **THEN** no warning appears
