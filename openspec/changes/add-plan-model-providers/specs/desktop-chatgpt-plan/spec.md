## ADDED Requirements

### Requirement: ChatGPT plan use is off until the user signs in

Desktop SHALL keep ChatGPT plan use disabled on every device until the user signs
in through OpenAI's official flow. Desktop SHALL use OpenAI's required wording: the
button reads "Continue with ChatGPT", the Settings section is titled "Use your
ChatGPT plan", and the first sign-in shows "You're using your ChatGPT plan" with a
"Got it" button, once for the account. That notice does not gate use.

#### Scenario: First start

- **WHEN** a user opens Settings on a device that never signed in
- **THEN** ChatGPT plan use is off and no ChatGPT model appears in the picker
- **AND** no request goes to `auth.openai.com` or `api.openai.com`

#### Scenario: First sign-in

- **WHEN** the user selects "Continue with ChatGPT" and completes sign-in
- **THEN** Desktop shows the first-sign-in notice once
- **AND** later sign-ins for the same account do not show it

### Requirement: Sign-in follows OpenAI's documented flow and keeps tokens private

Desktop SHALL register a new sign-in with `client_id=dynamic_agent_client`, the app
name and a stable opaque host id, and SHALL use PKCE S256 with a fresh state and
nonce per attempt. It SHALL receive the code on a loopback listener on `127.0.0.1`.
A returning sign-in SHALL use the saved issued client id. Desktop SHALL verify the
ID token's signature, issuer, audience, expiry and nonce, and SHALL require the
granted scope `chatgpt.tokens.use.direct` before any inference. It SHALL seal the
tokens on disk, one file per issued client id, under the account's key from the
secret store. It SHALL NOT read Codex's credential cache, start `codex`, or call
ChatGPT `backend-api` endpoints.

#### Scenario: A callback with the wrong state

- **WHEN** the callback state does not match the attempt
- **THEN** Desktop exchanges no code and stores nothing
- **AND** it answers 400 and keeps listening; twenty such callbacks end the sign-in

#### Scenario: The plan-usage scope is missing

- **WHEN** the granted scopes lack `chatgpt.tokens.use.direct`
- **THEN** Desktop keeps the sign-in, marks plan use disabled, and runs no inference
- **AND** it does not switch to another billing path

#### Scenario: Secrets stay out of logs

- **WHEN** Desktop signs in, refreshes, or signs out
- **THEN** no access token, refresh token, ID token, or authorization URL that carries `id_token_hint` appears in any log, event, or screen

### Requirement: Tokens refresh safely and sign-out revokes

Desktop SHALL refresh the access token ahead of expiry, serialize refreshes per
session, and save the new access token, expiry, scopes and rotated refresh token
together before it uses the new token. It SHALL treat `invalid_grant`,
`invalid_refresh_token`, `token_expired`, `refresh_token_expired`,
`refresh_token_invalidated` and `refresh_token_reused` as terminal: it SHALL delete
the sealed session for that client id and ask for a new sign-in with the saved
client id. It SHALL keep the sign-in on a network or 5xx failure. On sign-out it
SHALL delete the sealed session and clear the active client id, then post the
refresh token to the revocation endpoint.

#### Scenario: A rotating refresh token

- **WHEN** two requests need a fresh token at the same time
- **THEN** one refresh runs and both use its result

#### Scenario: Revocation is unconfirmed

- **WHEN** sign-out cannot confirm revocation
- **THEN** Desktop has already cleared the local record
- **AND** tells the user to disconnect the app in ChatGPT Settings

#### Scenario: The registration is refused

- **WHEN** a refresh is refused as `invalid_client`
- **THEN** Desktop deletes the sealed session and the profile
- **AND** the next sign-in registers a new client

### Requirement: The Responses adapter obeys OpenAI's request rules

Desktop SHALL call `POST https://api.openai.com/v1/responses` with `store: false`
and `stream: true`. It SHALL put the system text in a `developer` input item at the
start of `input`, never in a system-role item and never in `instructions`. It SHALL
resend the whole history in `input`, send only function tools, and omit every field
OpenAI lists as unsupported, `instructions` and `previous_response_id`. It SHALL
treat a stream that ends without `response.completed` as a failure. It SHALL return
every tool request to the existing tool loop.

#### Scenario: The model requests a tool

- **WHEN** a response contains `function_call` items
- **THEN** Desktop emits one complete tool call per item, then `Stopped` with `ToolUse` and `Completed`
- **AND** the existing tool loop runs the tools and sends the next request

#### Scenario: The usage limit is reached

- **WHEN** a response fails with `subscription_sharing_usage_limit_exceeded`
- **THEN** Desktop stops plan requests and shows "Usage limit reached" with a "Manage usage" link to `https://chatgpt.com/settings/usage`
- **AND** it shows no estimate or reset time and does not retry

#### Scenario: The token is refused

- **WHEN** a request fails with HTTP 401 or `subscription_sharing_invalid_user`
- **THEN** Desktop deletes the sealed session for the active sign-in and keeps the profile
- **AND** the status shows sign-in required, and the adapter does not refresh and retry

#### Scenario: The model refuses

- **WHEN** the stream carries `response.refusal.delta` events
- **THEN** Desktop shows the refusal text as output, and `response.refusal.done` adds nothing

#### Scenario: Transient unavailability

- **WHEN** a request fails with a 503 or `subscription_sharing_usage_unavailable`
- **THEN** Desktop keeps the credentials and retries with bounded backoff, at most three attempts and only before any output

### Requirement: Only documented ChatGPT models are offered

Desktop SHALL list models with `GET https://api.openai.com/v1/models`, keep entries
whose `visibility` is `list`, and offer only the documented slugs: `gpt-6-luna`,
shown as Luna, and `gpt-6.1-sol`, shown as Sol. The display names are Desktop's own,
whatever the listing calls them. Desktop SHALL record `chatgpt_plan/<slug>` as the
run model and `chatgpt_plan` as the provider, and show "Using ChatGPT plan" in the
composer while a ChatGPT model is selected.

#### Scenario: An undocumented slug

- **WHEN** the model list contains a slug with no documented limits
- **THEN** Desktop does not offer it. `gpt-6-astra` is not offered.
- **AND** a request for that slug is refused before any request body is built

#### Scenario: The account changes

- **WHEN** the signed-in ChatGPT account changes
- **THEN** the workspace refresh reloads the model list for the new account
- **AND** sign-out has already cleared the previous account's ChatGPT rows

### Requirement: Existing providers are unchanged

With ChatGPT plan use off, the model catalog, IPC surface, provider dispatch and
inference output of every existing provider SHALL be identical to the behavior
before this change.

#### Scenario: Plan use off

- **WHEN** ChatGPT plan use is disabled
- **THEN** the existing catalog, security and provider tests pass unchanged
- **AND** no listener opens and no request goes to OpenAI's sign-in origin

## Addendum 2026-10-22: conversation headers

The requirement below extends this change. It is dated 2026-10-22 and does not
replace the requirements above. The effect on cache hits and plan usage is not
measured live. It needs a signed-in ChatGPT plan that OpenAI has approved for this
app, so the effect is **UNVERIFIED**.

### Requirement: Plan requests carry the conversation id in routing headers

Desktop SHALL send the conversation id as the `session-id`, `thread-id` and
`x-client-request-id` headers of every `POST https://api.openai.com/v1/responses`
request. It SHALL accept an id only if it is an opaque UUID of 1 to 64 ASCII letters,
digits, `-` or `_`. It SHALL NOT change the request body, and it SHALL NOT send
`prompt_cache_key`.

#### Scenario: A chat turn

- **WHEN** a ChatGPT plan turn sends a request
- **THEN** the three headers carry the conversation id
- **AND** the request body is the same as before this change

#### Scenario: An invalid id

- **WHEN** the conversation id is not 1 to 64 ASCII letters, digits, `-` or `_`
- **THEN** Desktop does not send that id in the three headers
