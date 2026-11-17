# Office support review record

The first row group below came from the 2026-10-27 audits of the text readers. The second came from a later whole-commit review. The LibreOffice runtime that the first audits also covered was removed; its findings left with it and are in git history.

## Text readers and project integration

| Finding                                                                                                       | Correction                                                                                           | Regression evidence                                                                                               |
| ------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| Structurally valid blank or image-only files could be treated as corrupt.                                     | Preserve a valid no-text outcome through attachment and project paths.                               | Native parser, upload and project tests pass.                                                                     |
| Generated extraction-limit notices could match project search queries.                                        | Search only document-derived text, preserving reported line offsets.                                 | Project search tests pass, including notice-only queries.                                                         |
| DOCX header or footer references without relationship IDs could be silently skipped.                          | Reject malformed references.                                                                         | Parser negative fixtures pass.                                                                                    |
| Visual-preview errors could make extracted text unavailable, and stale previews could survive source changes. | Separate text and preview state; cancel superseded jobs and check source identity before publishing. | Mounted project viewer and IPC lifecycle tests pass.                                                              |
| PPT `TextBytesAtom` text was decoded as Latin-1, so curly quotes, dashes and the euro sign became spaces.     | Decode as Windows-1252, as LibreOffice's reader does.                                                | `ppt_text_bytes_atoms_decode_as_windows_1252`, which fails with the old decoding.                                 |
| Distinct PPT persist ids could point at one record, repeating every walk once per alias.                      | Reject slide and notes persist ids that share an offset.                                             | `rejects_slides_whose_persist_ids_share_one_record` (with a distinct-ids control), which fails without the check. |
| OOXML relationship lookup scanned the whole list for every reference.                                         | Index relationships by id once per part.                                                             | The existing chart, diagram and relationship tests pass.                                                          |

## Preview and save

| Finding                                                                                                                         | Correction                                                                                      | Evidence                                                                            |
| ------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| Word's `Close` and `Quit` need by-reference arguments in PowerShell; without them Office stayed running after every conversion. | Pass them as references.                                                                        | Checked by hand: no Word, PowerPoint or Excel process remained after repeated runs. |
| Releasing Excel's COM object by hand blocked for about a minute.                                                                | Do not release by hand; the process exit does.                                                  | Checked by hand: an Excel conversion takes seconds.                                 |
| A script file is refused by common execution-policy settings.                                                                   | Pass the script inline with `-EncodedCommand`.                                                  | Checked by hand on a PC whose policy blocks script files.                           |
| A Word or Excel program the person already had open could be hidden or closed.                                                  | Only hide, report and quit a program this run started; restore the automation-security setting. | Reviewed in the script; not yet exercised with Office open (task 2.6).              |

## Open gates

- Native macOS run and the Apple Events permission and entitlement (task 2.5).
- Packaged Windows run, with and without Office (task 2.4).
- Live chat upload through the deployed data-analyst service (task 1.7).
- Pre-existing dependency advisories are unchanged by this work.
