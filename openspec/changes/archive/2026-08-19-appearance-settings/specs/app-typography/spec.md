# app-typography — Spec Delta

## MODIFIED Requirements

### Requirement: One type ramp, scaled as a whole

The system MUST derive every type role in the token ramp from one
reading-size multiplier, so a size change moves every role together — a
ramp whose roles scale independently stops being a ramp.

The fluid reading size keeps its `clamp()` shape at every step, so the
window-width response survives the multiplier.

#### Scenario: Scaling preserves hierarchy

- **WHEN** the reading size changes
- **THEN** every type role changes proportionally and their ratios hold
