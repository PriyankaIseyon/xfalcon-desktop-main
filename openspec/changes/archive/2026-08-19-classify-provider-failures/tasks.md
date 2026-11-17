## 1. Taxonomy

- [x] 1.1 Add `PaymentRequired`, `PayloadTooLarge`, `TlsVerificationFailed` to
      `ProviderError` with `Display` text naming the remedy
- [x] 1.2 Map 402 and 413 in `classify_status_code`
- [x] 1.3 Classify a certificate verification failure off the transport error
      instead of collapsing it to `Unavailable`

## 2. Recovery

- [x] 2.1 `delay_before_retry` returns `None` for all three new variants
- [x] 2.2 Update every remaining exhaustive `match` on `ProviderError`

## 3. Tests

- [x] 3.1 402 and 413 classify to their own variants
- [x] 3.2 A deterministic failure is attempted exactly once
- [x] 3.3 429 and 503 still retry, proving the change did not widen
- [x] 3.4 A TLS verification failure classifies without a live bad certificate
