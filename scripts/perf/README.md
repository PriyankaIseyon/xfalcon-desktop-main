# Frontend performance benchmarks

Browser benchmarks for the production frontend. They measure cold startup, screen opens, interactions, streaming and scrolling against a mocked native (Tauri) boundary. Use them to compare two builds.

The scripts import `chromium` from `@playwright/test` (a devDependency). They are developer tools and are not part of the shipped app.

## What each script measures

- `startup.mjs`: cold page load. First contentful paint, time to the workspace shell, composer and first conversation row, script/style/layout time, long tasks, JS/CSS/font bytes.
- `screens.mjs`: click each destination (Projects, Datasets, Reports, Chats, Settings). Click to first paint, click to content visible, click to DOM settled, main-thread busy time, DOM nodes. Each destination is measured on first visit and on revisit.
- `scenarios.mjs`: interactions. Open a 200 and a 10,000 message conversation, scroll a 10k and a heavy-markdown transcript, scroll a 1,500-conversation sidebar, filter a 20k-row folder, open a CSV and filter it, type in the command palette on a 2,000 message chat, stream a 60 KB HTML answer, send a new chat with IPC latency.
- `profile.mjs`: CPU profile of one destination click, the `streamHtml` stream or the `scrollHeavy` scroll. Prints the top self-time functions.
- `compare.mjs`: prints the before/after table for two labels.
- `run.mjs`: runs `startup`, `screens` (plain and with IPC latency) and `scenarios` (all ten) into `scripts/perf/results/`.
- `mock.mjs`: the native boundary mock. `serve.mjs`: static server for a dist folder. `args.mjs`: shared flag parsing. `stats.mjs`: the median.

`scenarios.mjs` runs five scenarios by default: `open200,open10k,scroll10k,sidebar1500,filter20k`. Request others with `--only`, for example `--only csvOpenFilter,palette2k,streamHtml,newChatSend,scrollHeavy`, or list any combination. The ten names are `open200`, `open10k`, `scroll10k`, `sidebar1500`, `filter20k`, `csvOpenFilter`, `palette2k`, `streamHtml`, `newChatSend` and `scrollHeavy`. `run.mjs` passes all ten. A repeated name in `--only` runs once.

## Warm-up

Every script discards its first measured pass as a warm-up, so the browser's first-run costs stay out of the numbers:

- `startup.mjs`: one discarded page load, then `--runs` measured loads.
- `screens.mjs`: one discarded pass over the whole sequence, then `--runs` measured passes.
- `scenarios.mjs`: one discarded pass over all selected scenarios, then `--runs` measured passes.

## Flags

Every flag is checked before the server or the browser starts. A bad value prints one message and exits 1. A bad value is one of: a number that is negative, not finite, or below the minimum; a non-integer where an integer is required; an empty value; a flag name where a value should be. Every script resolves `--dist` against the working directory and fails with a message when the folder has no `index.html`.

`startup.mjs`

| Flag         | Default | Meaning                                                                                     |
| ------------ | ------- | ------------------------------------------------------------------------------------------- |
| `--dist`     | `dist`  | Vite build output folder                                                                    |
| `--runs`     | `9`     | Measured page loads after one discarded warm-up load. Integer, at least 1                   |
| `--throttle` | `4`     | CPU throttling rate, a number at least 1. `1` turns throttling off. Fractions are passed on |
| `--label`    | `run`   | Name written into the summary                                                               |
| `--port`     | `0`     | Server port, 0 to 65535. `0` picks a free port                                              |
| `--json`     | none    | Also write the summary and raw samples to this file                                         |

`screens.mjs`

| Flag              | Default                                                              | Meaning                                                                                                                                                                          |
| ----------------- | -------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--dist`          | `dist`                                                               | Vite build output folder                                                                                                                                                         |
| `--only`          | `Projects,Datasets,Reports,Chats,Projects,Datasets,Reports,Settings` | Destinations to click, in order, comma separated, not empty. Known names are `Projects`, `Datasets`, `Reports`, `Chats` and `Settings`. A repeated name is measured as a revisit |
| `--runs`          | `7`                                                                  | Measured passes after one discarded warm-up pass. Integer, at least 1                                                                                                            |
| `--throttle`      | `4`                                                                  | CPU throttling rate, a number at least 1. `1` turns throttling off                                                                                                               |
| `--label`         | `run`                                                                | Name written into the summary                                                                                                                                                    |
| `--ipcDelay`      | `0`                                                                  | Mock delay in ms added to each native command, a number at least 0                                                                                                               |
| `--ipcJitter`     | `0`                                                                  | Extra mock delay in ms, varied per command, a number at least 0                                                                                                                  |
| `--conversations` | `300`                                                                | Conversations in the mocked sidebar, integer at least 0                                                                                                                          |
| `--dirEntries`    | `4000`                                                               | Files in the mocked `data` folder, integer at least 0                                                                                                                            |
| `--datasets`      | `200`                                                                | Datasets in the mocked list, integer at least 0                                                                                                                                  |
| `--reports`       | `200`                                                                | Reports in the mocked list, integer at least 0                                                                                                                                   |
| `--port`          | `0`                                                                  | Server port, 0 to 65535. `0` picks a free port                                                                                                                                   |
| `--json`          | none                                                                 | Also write the summary and the raw samples to this file                                                                                                                          |

`scenarios.mjs`

| Flag         | Default                                           | Meaning                                                                          |
| ------------ | ------------------------------------------------- | -------------------------------------------------------------------------------- |
| `--dist`     | `dist`                                            | Vite build output folder                                                         |
| `--only`     | `open200,open10k,scroll10k,sidebar1500,filter20k` | Scenario names, comma separated, not empty. See the list above                   |
| `--runs`     | `5`                                               | Measured runs per scenario after one discarded warm-up pass. Integer, at least 1 |
| `--throttle` | `4`                                               | CPU throttling rate, a number at least 1. `1` turns throttling off               |
| `--label`    | `run`                                             | Name written into the summary                                                    |
| `--port`     | `0`                                               | Server port, 0 to 65535. `0` picks a free port                                   |
| `--json`     | none                                              | Also write the summary and the raw samples per scenario to this file             |

`profile.mjs`

| Flag         | Default    | Meaning                                                                                                                    |
| ------------ | ---------- | -------------------------------------------------------------------------------------------------------------------------- |
| `--dist`     | `dist`     | Vite build output folder                                                                                                   |
| `--dest`     | `Datasets` | A destination name (`Projects`, `Datasets`, `Reports`, `Chats`, `Settings`), `streamHtml` or `scrollHeavy`                 |
| `--throttle` | `1`        | CPU throttling rate, a number at least 1. `1` turns throttling off                                                         |
| `--wait`     | `3000`     | Milliseconds to profile after a destination click, a number at least 0. Destinations only                                  |
| `--cfg`      | `{}`       | JSON object of mock options from `mock.mjs`, for example `'{"datasets":200}'`. A key that is not a mock option is an error |
| `--top`      | `22`       | Number of functions in the printed list. Integer, at least 1                                                               |
| `--port`     | `0`        | Server port, 0 to 65535. `0` picks a free port                                                                             |
| `--json`     | none       | Write the raw CPU profile to this file                                                                                     |

`run.mjs`

| Flag      | Default         | Meaning                                                             |
| --------- | --------------- | ------------------------------------------------------------------- |
| `--dist`  | `dist`          | Vite build output folder                                            |
| `--label` | none (required) | Name for the result files. Letters, digits, `.`, `_` and `-`        |
| `--runs`  | `9`             | Passed to `startup`, `screens` and `scenarios`. Integer, at least 1 |

`compare.mjs` takes no flags. It takes two labels, each with the same letters, digits, `.`, `_` and `-` as `run.mjs`: `node scripts/perf/compare.mjs <labelA> <labelB>`.

## Run before and after

`npm run build` is not needed. Build each revision to its own folder:

```sh
# in a checkout of the before revision (a git worktree keeps your working tree intact)
npx vite build --outDir <dist-a> --emptyOutDir
node scripts/perf/run.mjs --dist <dist-a> --label before

# in a checkout of the after revision
npx vite build --outDir <dist-b> --emptyOutDir
node scripts/perf/run.mjs --dist <dist-b> --label after

npm run perf:compare -- before after
```

`npm run perf` is the same as `node scripts/perf/run.mjs`, so `npm run perf -- --dist <dist-a> --label before` also works. A full run takes several minutes.

`run.mjs` runs four steps and writes four files per label to `scripts/perf/results/`:

| Step                     | Command                                     | Output file                                              |
| ------------------------ | ------------------------------------------- | -------------------------------------------------------- |
| startup                  | `startup.mjs`                               | `<label>.startup.json`                                   |
| screens                  | `screens.mjs`                               | `<label>.screens.json`                                   |
| screens with IPC latency | `screens.mjs --ipcDelay 120 --ipcJitter 80` | `<label>.screens-lat.json` (summary label `<label>-lat`) |
| scenarios                | `scenarios.mjs` with all ten scenarios      | `<label>.scenarios.json`                                 |

After the last step, `run.mjs` prints one warning line listing every native command the mock did not handle, from the four files. It does not fail the run. `run.mjs` also forwards SIGINT and SIGTERM to the running step and exits with that step's status (130 or 143 when it was stopped).

`profile.mjs` resolves frames to source only when the build has sourcemaps (`npx vite build --sourcemap`). Without them, frames show bundle lines.

The first run needs Playwright's Chromium (`npx playwright install chromium`).

## Output

- `startup.mjs`, `screens.mjs` and `scenarios.mjs` print a JSON summary to stdout. `--json` also writes the raw samples. `startup.mjs` prints `medians`, `p90` and `static`. `screens.mjs` prints `screens` keyed by destination and visit (`Projects:first`, `Projects:revisit`), with `paintMs`, `contentMs`, `settledMs`, `busyMs` and `nodes`. `scenarios.mjs` prints `results` per scenario, with the median of each numeric value.
- Medians of an even number of runs average the two middle values.
- `contentMs` is the median over the runs where the marker text appeared. A run where it did not appear is recorded as `null` in the `--json` samples and left out of the median. If a marker never appears in any measured run, the script fails with a message and writes no summary.
- `scenarios.mjs` fails with a message when a scroller or a marker is missing, within its own deadline (60 seconds for rendered rows, 60 seconds for the element waits). It never writes an empty or partial result.
- `scenarios.mjs` `newChatSend.sendIssuedMs` is the time from the key press to the moment `chat_message_send` reached the mock backend. The mock stamps it after the simulated IPC delay (120 ms here). It therefore includes that delay for every native call the app awaited before the send. The JSON key is unchanged.
- `profile.mjs` prints a JSON summary: `dest`, `throttle`, `busyMs`, `sourcemap` and `top`. Each sample is charged the CPU time until the next sample starts, so the last sample is charged zero. `--json` writes the raw CPU profile instead, which opens in the Chrome DevTools profiler.
- `compare.mjs` prints a text table. Each row shows the value for each label and the percentage change, `(after - before) / before`.
- Every script that drives the browser prints `unknownCommands`: the sorted, de-duplicated list of native commands the mock did not handle, collected across the runs.

On failure a script prints one message and exits 1. Chromium and the server are closed first, so nothing is left running. Flag errors are reported before anything starts.

## How to read the numbers

- The numbers come from headless Chromium with a 4x CPU throttle and a mocked native boundary. They are a relative signal for the frontend only. They do not include WebView2, the network or the Rust side. Do not compare them with absolute user timings.
- Run both builds back to back on a quiet machine. Alternate runs (before, after, before, after) when a change is small.
- Use medians of 9 or more runs. Look for changes that repeat across runs and across the plain and IPC-latency screen runs.

## Ports

Every script binds `127.0.0.1` on a port the OS picks, so two scripts can run at the same time. Pass `--port` only to pin a port. A busy port fails with `EADDRINUSE` and exits 1.

The static server only serves files under the dist folder. It answers `Range` requests with the whole file (status 200), which Playwright page loads do not need. A missing file with an extension is a 404. A path without an extension falls back to `index.html`.

## Mock limits

- IPC is instant unless `--ipcDelay` is set. The plain runs therefore hide the cost of waiting for the native side. The `screens-lat` run uses 120 ms +- 80 ms (`--ipcDelay 120 --ipcJitter 80`). `newChatSend` uses a fixed 120 ms.
- Responses are fixed fixtures. The project tree, CSV and transcripts are generated, not real data. `report_thumbnail` returns a 1x1 PNG and `chat_follow_ups` returns two fixed suggestions.
- Inference streaming is simulated by emitting events into the page. Network, model latency and the Rust event pipeline are not measured.
- Commands the mock does not know resolve to `null`. Check `unknownCommands` when a new command is added, because the screen or scenario measured against that `null`.
