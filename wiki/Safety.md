# Safety

The goal is that using this tool on your car is no riskier than leaving it plugged in with nothing running.

## What the scanner is allowed to send

Only a short, fixed list of diagnostic requests: start a standard diagnostic session, keep it alive, read data, read
trouble codes. Anything else is refused inside the program before it reaches the adapter. Requests are never sent to
"every control unit at once" except the harmless wake-up probe.

## What the first release does not do

- It does **not** clear trouble codes.
- It does **not** run actuator (bi-directional) tests such as fans or clutches.
- It does **not** write anything: no adaptation resets, no learn values, no calibration, no reflash.

These exist in development and have never been run on a real car. They will not be offered until they have been, with
strict simulators, a confirmation at every step, your VIN matched, and the conditions the original tool checks.

## Reading is not free

A diagnostic tool still shares the bus with your car. Keep the car stationary while scanning, keep the battery healthy, and
do not unplug the adapter mid-scan. If the dash shows a warning that was not there before, turn the ignition off and on.

## Your data

Nothing leaves your machine unless you turn on [sharing](Privacy-and-Sharing.md).

## Disclaimer

Use at your own risk, on your own vehicle. See the disclaimer in the [README](../README.md).
