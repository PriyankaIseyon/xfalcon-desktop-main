## ADDED Requirements

### Requirement: Local-only commands require a session that owns the account

A command SHALL run only when the account bound on the device belongs to an
authenticated session that can still act for it, if the command:

- serves what the device holds for an account, and
- sends no bearer to a server that would check it.

These commands are the project commands (listing, trust, activation, directory
and file reads, schema summary, saves, revisions, restores, and patch
decisions), the report list and open, and conversation search and preview. The
bound account alone SHALL NOT be sufficient.

A refusal SHALL use the error each surface already returns to a signed-out
device, so the frontend needs no new handling.

#### Scenario: The session ends without a sign-out

- **GIVEN** a signed-in account whose session is then cleared by a refused
  renewal, a cancelled sign-in, or a failed sign-in, leaving the account bound
- **WHEN** a project, report-read, or conversation-search command is invoked
- **THEN** it is refused with that surface's signed-out error
- **AND** no project file is read or written and no cached report is returned

#### Scenario: A signed-in account

- **WHEN** a signed-in user uses projects, reports, or search
- **THEN** every command behaves as it did before this requirement

### Requirement: A dead session acts for no account

A session SHALL be treated as unable to act for its account once it is past
expiry by more than the token clock tolerance **and** holds no refresh token.

A session that is past expiry but still holds a refresh token SHALL continue to
act for its account. It renews on the next request that reaches the tenant.

#### Scenario: Expired with no way to renew

- **GIVEN** a session an hour past expiry with no refresh token
- **WHEN** a local-only command is invoked
- **THEN** it is refused, and conversation search and preview report the
  session as expired

#### Scenario: Expired while offline, renewable

- **GIVEN** a session an hour past expiry that holds a refresh token
- **WHEN** a local-only command is invoked with no connection
- **THEN** it runs as before

#### Scenario: Inside the clock tolerance

- **GIVEN** a session thirty seconds past expiry with no refresh token
- **WHEN** a local-only command is invoked
- **THEN** it runs, because the tenant would still accept the bearer
