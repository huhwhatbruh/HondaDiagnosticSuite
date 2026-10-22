# Getting started

> **To be written** once the first binary release exists. This is the outline it will follow.

## 1. Download

Releases will be on the [Releases page](../../../releases) as one archive per platform plus a `.sha256` file. Check it:

```
sha256sum -c hondascan-<version>-linux-x86_64.tar.gz.sha256
tar xzf hondascan-<version>-linux-x86_64.tar.gz
```

The archive holds the desktop app (`hondascan-gui`), its backend (`hondascan-rpc`), the command-line tool (`hondascan`)
and the data it needs, side by side. Nothing else needs to be installed: no Python, no Rust.

## 2. Let your user talk to the adapter (Linux)

Copy the udev rule that ships in the archive so you do not need root, then replug the adapter:

```
sudo cp 60-hondascan.rules /etc/udev/rules.d/ && sudo udevadm control --reload
```

Add yourself to the serial group (`uucp` on Arch, `dialout` on Debian and Ubuntu). The GODIAG GD101 in J2534 mode also
needs a C compiler (`cc`, `gcc` or `clang`) once, to build its small driver.

## 3. Try it without a car

```
hondascan -a sim scan
```

The simulated car answers like a real one, so you can look around the app before touching hardware.

## 4. First scan on a real car

1. Plug the adapter into the OBD-II port, ignition on (engine running is fine for reading).
2. Connect from the app's top bar, or run `hondascan -a <adapter> scan`. Adapter names are on the [Adapters](Adapters.md) page.
3. Read codes: the app's Trouble codes tab, or `hondascan -a <adapter> dtc`.

If something does not work, that is useful information: open a [vehicle report](../../../issues/new?template=vehicle_report.md).
