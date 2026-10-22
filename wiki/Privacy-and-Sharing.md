# Sharing diagnostics: what is sent and what is not

Sharing is **off by default**. The app never sends anything unless you turn it on, and even then each send asks you
first. You can read the exact text before it goes (`hondascan telemetry preview`, or "Show what would be sent" on the
History page), turn sharing off at any time, or skip uploading and attach the same file to an issue
(`hondascan telemetry export`).

## What a bundle holds

| Item | Detail |
|---|---|
| Car | model prefix (WMI), the descriptor part of the VIN, model year, plant. **Not the VIN**: the serial part is replaced by a hash salted with a random id that is yours alone, so two people reading the same car cannot be linked and the VIN cannot be recovered from the hash. |
| Control units | ECU ids, part numbers, system names |
| Trouble codes | code, system, first and last time seen, status |
| Sessions | start and end time, adapter type, how it ended, the session summary |
| Software | app version, operating system, CPU architecture |
| Install id | a random 128-bit id created when you turn sharing on and deleted when you turn it off |
| Logs | **only if you tick "Include session logs"**: the last 200 lines (at most 64 KiB) of the newest three sessions' journals |

Every text field is scrubbed before it is shown to you: VINs become `<vin>`, your data folder `<data>`, your home folder
`~`.

## What is never sent

Your VIN, your name or user name, your home folder, service-marker notes, the write audit trail, reports, and anything
about your computer beyond the operating system and architecture.

## What it is used for

Finding out which cars, control units and adapters work, which codes and refusals occur, and fixing bugs. It is not sold
or shared.

## Where it goes and for how long

To a write-only collection endpoint that stores each bundle in a private bucket, readable only by the project owner.
**Retention period: to be confirmed before the first release.**

## Deleting your data

Turning sharing off deletes your install id from your machine. To have bundles already sent removed, open an issue with
your install id (shown by `hondascan telemetry status` while sharing is on; note it before turning sharing off) and they
will be deleted.
