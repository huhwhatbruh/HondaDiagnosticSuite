# Supported vehicles

"Confirmed" means it was run on a real car and the results were checked. Everything else is **untested**, not unsupported:
the scanner discovers control units by asking for them, so many cars may work. Reports fill this table in.

| Vehicle | Engine | Adapter | What works | Status |
|---|---|---|---|---|
| 2015 Honda Accord | 2.4 L (K24W1), CVT | GODIAG GD101, J2534 mode | Scan (engine/CVT, SRS, meter), trouble codes, freeze frame, emissions readiness, live data, 48-minute recorded drive | **Confirmed** |
| Other Hondas on CAN | | | | untested |
| Older Hondas on K-line | | | Generic OBD-II over ISO 9141-2 is implemented; Honda-specific K-line systems are not complete | untested |

## Known limits on the confirmed car

- The scan finds the engine/CVT computer, SRS and instrument cluster. The ABS and EPS stay silent on this car, so they are not readable yet.
- Some live-data groups are refused by the car (these are shown as refused, not as faults).
- Reading the engine computer's memory is refused by the car, so engine-map work uses what the car reports while driving.

## Add your car

Run a scan, export a bundle and open a [vehicle report](../../../issues/new?template=vehicle_report.md). See
[Contributing a vehicle](Contributing-a-Vehicle.md).
