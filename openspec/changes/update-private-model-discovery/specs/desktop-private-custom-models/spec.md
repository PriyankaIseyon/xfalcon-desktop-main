## ADDED Requirements

### Requirement: Private model setup discovers models from the supplied API base

Desktop SHALL begin model creation with an HTTPS API base URL and key. Next SHALL issue a bounded authenticated native GET to that API base's `/models` path. Discovery SHALL use the existing public-address, DNS-pinning, TLS, and no-redirect policy. Discovery SHALL neither store the key nor run inference.

#### Scenario: The endpoint lists models

- **WHEN** the endpoint returns a valid model list
- **THEN** the user can choose an exact returned ID
- **AND** Desktop preserves any provider prefix without substituting a different model

#### Scenario: Discovery is optional

- **WHEN** the endpoint has no listing route, returns no models, returns malformed data, or cannot be reached
- **THEN** Desktop explains the reason and offers manual model entry
- **AND** a key rejection or address refusal remains an explicit error

#### Scenario: Discovery completes after cancellation or an account change

- **WHEN** the result arrives for an editor or account that is no longer current
- **THEN** Desktop ignores the result and does not expose the prior account's model data

### Requirement: Users can review discovered settings and supply missing values

Desktop SHALL preserve valid endpoint metadata exactly and label its source. Users SHALL be able to set context and output limits, tool/image capabilities, and model/display names. Missing settings SHALL use labeled xFalcon suggestions and ask the user to check them. Saved settings SHALL be preserved when editing. Model-name guidance SHALL explain exact provider-qualified IDs. Output limits SHALL leave room for input and reserved tokens within the context length.

#### Scenario: The catalog reports only IDs

- **WHEN** a model lacks limits and capabilities
- **THEN** Desktop suggests 32,768 context tokens, 4,096 output tokens, Temperature 0.7, and Top P 0.9 with clear labels and asks the user to check them and confirm capabilities
- **AND** it does not claim these values are model metadata or infer them from the model name

#### Scenario: The catalog reports only one token limit

- **WHEN** the endpoint reports only the context or output limit
- **THEN** Desktop preserves that reported value exactly and fits the other token suggestion alongside it
- **AND** a reported limit too small for xFalcon remains an explicit validation error

#### Scenario: The catalog reports settings

- **WHEN** a model reports valid limits or supported sampling defaults, including zero
- **THEN** Desktop uses the exact reported values and labels them as provided by the model endpoint
- **AND** it does not reduce reported capacity by 10% or replace unsupported sampling with suggestions

#### Scenario: Token limits cannot support an xFalcon request

- **WHEN** the configured context cannot fit positive input, output capped at 128,000 tokens, and the 16,384-token reserve, or raw output is not below context
- **THEN** Desktop shows an actionable field error and blocks save before inference or mutation
- **AND** correcting either token field reevaluates the pair and clears obsolete errors

#### Scenario: The user corrects or dismisses a failed save

- **WHEN** the user edits a field or dismisses a save error
- **THEN** the obsolete save error disappears and the reviewed settings remain available
- **AND** re-entering the API key keeps the field visible until the correction is complete

### Requirement: Duplicate private connections are blocked in the add dialog

Desktop SHALL block duplicate connections by normalized API URL, exact API key, and exact upstream model ID, regardless of display name. Native code SHALL check only the current account's exact bound keys and return no secrets. The dialog SHALL name the existing entry. Native saves SHALL recheck under the save lock before inference or mutation.

#### Scenario: A duplicate has a different display name

- **WHEN** the selected or manually entered upstream ID matches an existing connection with the same normalized URL and key
- **THEN** the add dialog explains that the model is already added under its existing name
- **AND** changing the display name does not permit saving

#### Scenario: The user changes the connection

- **WHEN** the user changes the URL or key and runs discovery again
- **THEN** Desktop checks uniqueness for that connection
- **AND** the previous connection's duplicate result does not block the new connection

#### Scenario: Duplicate preflight fails

- **WHEN** the catalog or an exact bound key cannot be checked
- **THEN** Desktop does not infer that the model is unique or mutate a model
- **AND** the editor remains open with its settings retained and a safe retry message

#### Scenario: An existing entry is edited

- **WHEN** the user updates an existing private model
- **THEN** Desktop excludes that row from the duplicate check
- **AND** another row with the same connection still blocks the update

### Requirement: Sampling settings are account-owned device preferences

Desktop SHALL offer temperature and Top P within their supported ranges. Unset values SHALL use the provider default. Explicit values SHALL be persisted per account and model on this device and sent by private inference. Unsupported discovered sampling parameters SHALL not be prefilled or sent. Storage errors SHALL not be reported as a successful save.

#### Scenario: Overrides are cleared

- **WHEN** the user clears temperature and Top P
- **THEN** subsequent private inference omits those fields
- **AND** another account's preferences remain unchanged

### Requirement: Connection checks precede saving a private model

Desktop SHALL require a completed, nonempty greeting before creation, connection, output-token or sampling changes, or re-enabling. The native save command SHALL check before DA or local model/key mutations. It SHALL use the guarded adapter, bound the whole operation and output, and send no tools or images. Failure SHALL keep the editor open with a safe correction message. Basic inference success SHALL not imply verified limits or capabilities.

#### Scenario: The connection succeeds

- **WHEN** the selected model completes the greeting using its exact model ID, key, and sampling settings
- **THEN** Desktop can save the reviewed model configuration

#### Scenario: The connection fails

- **WHEN** authentication, configuration, transport, timeout, empty output, or incomplete output prevents a completed greeting
- **THEN** Desktop does not change the saved model or key
- **AND** the editor explains the failure and allows the user to correct the connection

#### Scenario: An existing model uses its stored key

- **WHEN** a connection check is needed and the user leaves the key blank
- **THEN** Desktop uses only that model's bound stored key
- **AND** a missing or mismatched binding requires the user to enter the key again

#### Scenario: A change does not affect inference access

- **WHEN** the user changes only display, context-limit, or capability metadata, disables the model, or removes it
- **THEN** Desktop applies the existing mutation without a greeting request

### Requirement: The company profile does not change other selected models

Desktop SHALL apply the known company Qwen transport profile only to its exact gateway host and upstream model ID. It SHALL not infer missing model settings from that profile. Other private models, including models at the same host, SHALL retain their selected route and configured values.

#### Scenario: Another model shares the gateway

- **WHEN** the selected upstream model ID differs from the known Qwen ID
- **THEN** Desktop sends the selected ID and does not replace it with Qwen
