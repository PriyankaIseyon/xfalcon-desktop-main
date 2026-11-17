## 1. Stop destroying the other client's runs

Landed ahead of this proposal as a defect fix: the behaviour it removes
contradicted `chat-run-recovery`'s existing journal-tail scope, so restoring
that scope needed no new requirement.

- [x] 1.1 Classify an in-flight run with no local journal entry as foreign, in
      the same pass that already loads the journal for the interrupt patch.
- [x] 1.2 Exempt foreign runs from zombie finalization, so no
      `PATCH /runs/{id}/status` `cancelled` is issued for them.
- [x] 1.3 Exempt foreign runs from the recovery banner, so another client's
      healthy answer is not reported as this device's interruption.
- [x] 1.4 Split `finalize_abandoned_runs` out of `workspace_bootstrap` so the
      DA boundary is assertable, mirroring
      `cleanup_deleted_projects_for_account`.
- [x] 1.5 Prove at the DA boundary that a foreign run — running, and awaiting
      approval — produces zero requests, that an abandoned run still produces
      exactly one `cancelled` status PATCH, and that the two are separated
      correctly when both are present in one conversation.
- [x] 1.6 Prove the ownership rule itself against a real journal on disk, not
      against a literal: a run with no journal file is another client's, a run
      with an appended transition is this device's.
- [x] 1.7 Keep a foreign run out of the recovery run set, not merely out of the
      active-run slot. `startup_recovery_for_states` answers "something in
      flight, nothing active" with an unconditional notice, so excluding it as
      the active run alone still raised a crash banner for another client's
      healthy answer.
- [x] 1.8 Keep a foreign run from becoming the view's active run or the
      reported run state, on both the workspace bootstrap and
      `open_and_emit`. Without a follower, presenting it as active holds the
      composer on a spinner this window can never end — the regression the
      old cancellation was accidentally masking.
- [x] 1.9 Derive the foreign set once, in `ChatService::foreign_run_ids`, so
      the bootstrap and the open path cannot drift apart on the rule that
      decides whether a durable cancel is sent.

## 2. Read a foreign run's events

- [ ] 2.1 Add a `RunEventSource` seam with one operation, "events after this
      sequence", implemented over `DaChatSession::list_run_events`.
- [ ] 2.2 Map a `RunEvent` page to the projection events a locally executed run
      produces, so a followed transcript and a local one are one shape.
- [ ] 2.3 Prove the mapping against a wiremock event page, including a page
      that repeats sequences already projected.

## 3. Follow lifecycle

- [ ] 3.1 Start a follower when reconcile finds an in-flight foreign run in the
      open conversation; never start one for a run this process drives.
- [ ] 3.2 Poll at 500 ms focused, 2 s unfocused, and not at all when no foreign
      run is in flight.
- [ ] 3.3 Stop on terminal state, conversation close, account change, and
      sign-out; prove no poll survives any of the four.
- [ ] 3.4 Reconcile once on terminal so the durable assistant message replaces
      the followed stream without duplicating it.
- [ ] 3.5 Prove `drives_run` still answers false for a followed run, so boot
      recovery never reclaims it.

## 4. Present a followed run honestly

- [ ] 4.1 Separate "run is live" from "this process owns its cancellation" in
      `runState`/`chatReduce`.
- [ ] 4.2 Stream followed text and thinking into the transcript as a local run
      streams.
- [ ] 4.3 Keep Stop available for a followed run; keep retry and revise
      disabled until terminal.
- [ ] 4.4 Report a followed run that stops producing events as stalled, with
      Stop offered and no automatic cancellation.
- [ ] 4.5 Frontend tests for each of the above, including the transition from
      followed stream to durable message.

## 5. Close the loop

- [ ] 5.1 Update `docs/runtime-flows.md` and `docs/decisions.md` with the
      two-client run model and the poll-not-push constraint.
- [ ] 5.2 Update `docs/status.md`: the desktop is a conforming run-event writer
      and now also a reader; record the orphan-reaping question as DA's.
- [ ] 5.3 Live check against staging with a web session: start a turn on web,
      open the desktop, confirm the answer continues and finishes in both.
