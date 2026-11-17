## 1. Specification

- [x] 1.1 Define one response-scoped durable activity section.
- [x] 1.2 Define live suppression of intermediate tool-round narration.

## 2. Implementation

- [x] 2.1 Aggregate durable tool calls across all tool rounds in a response.
- [x] 2.2 Render only the final assistant answer after the activity section.
- [x] 2.3 Apply the same order to the live timeline.

## 3. Verification

- [x] 3.1 Prove multi-round durable responses render one collapsed section.
- [x] 3.2 Prove live responses render one activity section and one answer.
- [x] 3.3 Pass typecheck and targeted chat tests.
