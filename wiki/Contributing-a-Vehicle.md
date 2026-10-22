# Contributing a vehicle

The most useful thing you can do is run the tool on a Honda that is not on the [supported list](Supported-Vehicles.md) and
tell us what happened, including when nothing worked.

## What you need

- A Honda with an OBD-II port and a supported [adapter](Adapters.md).
- The car parked, ignition on. Engine running is fine for reading.

## Steps

1. Scan: `hondascan -a <adapter> scan`. Note which systems were found.
2. Read codes: `hondascan -a <adapter> dtc`.
3. Optional: record a short drive (the app does this automatically per session).
4. Export a bundle you can attach: `hondascan telemetry export --out bundle.json`.
   Open the file and read it first; it has no VIN, name or home folder in it. Add `--logs` for more detail.
5. Open a [vehicle report](../../../issues/new?template=vehicle_report.md), fill in the car and what worked, and attach the file.

If you would rather help directly, say so in the issue and you can be added to the private repository with the source code.

## If something fails

Failures are results too. Say what you ran, what you expected and what the tool printed. A capture of the raw bus
(`--log capture.log`) lets the problem be replayed without your car.
