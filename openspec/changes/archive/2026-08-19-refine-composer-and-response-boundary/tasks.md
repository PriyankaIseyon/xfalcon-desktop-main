# Tasks

## 1. Composer

- [x] 1.1 `resizeToContent`: measure from a collapsed box, cap at 300px,
      and drive `overflow-y` from the measurement — `hidden` below the cap,
      `auto` at or past it.
- [x] 1.2 Pin the input's line box to an integer (`leading-6`) and drop
      the standing `overflow-y-auto` from its class list.
- [x] 1.3 Placeholder → "Ask about your data"; update the composer test
      that pins the old copy.
- [x] 1.4 Test: scrollbar-free at one line, `auto` + 300px past the cap.

## 2. Response boundary

- [x] 2.1 `ResponseRibbon.tsx` — label + accent hairline, static,
      `aria-hidden`.
- [x] 2.2 `MessageRow` renders it above the assistant prose only, on the
      stream row and the durable row alike.
- [x] 2.3 `.response-ribbon` / `__label` / `__rule` in `index.css`, on
      `--xf-` tokens rather than web's hard-coded slate.
- [x] 2.4 Tests: present on assistant turns, absent on user turns,
      present on the in-flight row.

## 3. Accent rail

- [x] 3.1 `.assistant-accent*::before` width `1px` → `.5px`; drop the
      now-meaningless `border-radius`.

## 4. Verification

- [x] 4.1 `vitest run` green.
- [x] 4.2 `tsc -b` and `eslint` clean on the changed files.
- [x] 4.3 Rebuild and screenshot the running app: empty composer with no
      scrollbar, ribbon above the answer, hairline rail.
