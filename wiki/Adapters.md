# Adapters

An adapter is named with `-a` on the command line (the app has a picker with the same names).

| Name | Hardware |
|---|---|
| `socketcan:can0` | Anything with a Linux SocketCAN driver: candleLight/gs_usb (CANable and many USB-CAN sticks), PEAK, Kvaser, `slcand`-attached serial adapters |
| `slcan:/dev/ttyACM0` | Lawicel/slcan serial adapters directly |
| `j2534:/path/to/lib.so` | Any SAE J2534 pass-through device that has a Linux library |
| `gd101-j2534` | GODIAG GD101 in J2534 mode, through the project's native Linux driver (**confirmed on a car**) |
| `gd101` | GODIAG GD101 in ELM mode |
| `elm:/dev/ttyUSB0` | ELM327 and compatible clones, STN11xx / OBDLink |
| `elm-obd9141`, `gd101-obd9141` | Generic OBD-II on the K-line (ISO 9141-2) for older cars |
| `sim` | Built-in simulated car, no hardware |
| `replay:FILE` | Replays a recorded capture, no hardware |

`hondascan ports` lists the USB serial adapters it can see; `hondascan -a <adapter> adapter` shows the adapter's version
and the car's battery voltage.

## GODIAG GD101

Supported in both modes with no Wine and no Windows driver. After using the adapter with the vendor's Windows software,
unplug and replug it before using it here.

## Cheap ELM327 clones

They often work but are the least predictable; a clone that cannot do flow control may need a retry. Report what you see.

**To be written:** a per-adapter table of tested units, firmware versions and quirks.
