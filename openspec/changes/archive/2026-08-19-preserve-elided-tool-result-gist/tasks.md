## 1. Elision

- [x] 1.1 Replace `CLEARED_TOOL_RESULT` with a bounded excerpt plus notice
- [x] 1.2 Cut the excerpt on a UTF-8 character boundary
- [x] 1.3 Judge eligibility against the elided length, not a constant, so each
      call still strictly reduces `total_bytes`

## 2. Tests

- [x] 2.1 A long result keeps its opening and says the rest was dropped
- [x] 2.2 A multibyte cut point yields valid UTF-8
- [x] 2.3 An already-elided result is ineligible, so the loop terminates
- [x] 2.4 A result at or under the elided length is left intact
