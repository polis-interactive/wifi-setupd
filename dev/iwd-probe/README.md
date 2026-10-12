# iwd adapter probe (development only)

This separate Cargo workspace depends on the product adapter; product crates
never depend on or invoke this probe. It is excluded from product packages and
production image installation. Deployment uses `dev/pi/pi.py`'s verified
Ethernet transport and target identity checks, but installs a separate binary
at `/data/bin/wifi-setupd-iwd-probe`. It does not stop or overwrite the product
app. `--config` and `--remote-probe` parameterize the target and probe location.

From the repository root:

```sh
python3 dev/pi/probe.py build
python3 dev/pi/probe.py deploy
python3 dev/pi/probe.py status
python3 dev/pi/probe.py scan
python3 dev/pi/probe.py known
```

The build uses the configured Buildroot cross-linker, committed lockfile, and
AArch64 musl target. Upload checks the ELF, length, and SHA-256 before atomic
activation. SSH host keys and Ethernet transport are checked on every command.

## Station and replacement checks

Your normal Wi-Fi network is suitable for a basic connection check; keep its
working profile saved in iwd. For destructive replacement/deletion checks, use
a disposable profile or explicitly choose to discard that Pi-side profile.
Supply its password
only to the hidden local terminal prompt; the wrapper forwards it over an
anonymous stdin pipe and encrypted SSH, never arguments or files:

```sh
python3 dev/pi/probe.py connect 'YOUR TEST SSID'
python3 dev/pi/probe.py status
python3 dev/pi/probe.py connect-known 'YOUR TEST SSID'
```

`connect` submits new credentials and refuses an existing matching profile.
`connect-known` lets iwd use its existing profile without reading its secrets.
Check `Ready` against `python3 dev/pi/pi.py wifi-status`; the adapter requires
iwd station connectivity plus a usable IPv4 address on the named Wi-Fi interface.
No ping or Internet access is required for Ready. Run station checks separately
with WPA2, WPA3-only, and mixed mode where the test AP supports these settings;
`Personal` in scan results alone does not identify the negotiated security.

For explicit overwrite of that same profile:

```sh
python3 dev/pi/probe.py replace 'YOUR TEST SSID'
```

The local prompt confirms replacement. The adapter calls iwd Forget, then
Connect with a temporary authentication agent. Failed replacement leaves the
old profile discarded. It does not copy secrets or restore the old password.
Test a changed router password and a deliberately wrong replacement separately.
Record sanitized outcomes; never copy profile contents or password prompts into
logs. No automatic assertion that a generic iwd `Failed` means bad credentials.

For a profile selected for disposable testing, cleanup is an explicit, scoped
part of the test, including after failure. Do not run this cleanup on the normal
working network unless its removal was requested:

```sh
python3 dev/pi/probe.py disconnect
python3 dev/pi/probe.py forget 'YOUR TEST SSID'
python3 dev/pi/probe.py known
```

If cleanup fails, report the test as failed/pending and rerun scoped cleanup;
never remove `/var/lib/iwd` or unrelated profiles. Pass `open` after the SSID to
`connect-known`/`forget` for an open network; `enterprise` selects an iwd 802.1X
profile for metadata/deletion or connection using its existing configuration.
New enterprise credentials are not supported. The default is `personal`.

## Phone/AP check

```sh
python3 dev/pi/probe.py ap wifi-setupd-test
```

Enter a temporary 8–63 character AP passphrase. Join the AP on the phone, accept
that it has no Internet, and record the phone's IPv4 address. Verify the phone
address and the Pi's Wi-Fi AP address independently. There is no HTTP portal yet.
The command returns after startup; iwd keeps the AP running until an explicit
`stop-ap` command or another requested radio transition. There is no shutdown
timer, and closing the terminal does not stop the AP. The eventual setup AP
remains available until connection succeeds or provisioning is explicitly stopped.
On this Pi, iwd's default HT channel configuration is rejected by brcmfmac
(`START_AP failed: -52`). The Python `ap` command creates a temporary, mode-0600
iwd AP profile with `DisableHT=true` and calls the adapter's `StartProfile` path.
It derives a WPA2 PSK locally and sends it through SSH stdin; the secret appears
only in iwd's scoped `/var/lib/iwd/ap/<SSID>.ap` profile, which is removed after
iwd has loaded it and startup completes. iwd retains the active AP configuration
in memory. An existing profile is never overwritten. This command
rejects SSIDs containing slashes because iwd uses the SSID as the filename.
The passphrase prompt explicitly refers to the **Pi AP**, not the router.

The product adapter can either supply AP credentials to `Start` or reference
an image-provided iwd profile using `StartProfile`; it never reads or writes
credential files. Production image AP configuration remains in BuildrootImages.
The choice between an open and protected setup network remains to be decided.
Stop the AP and return the radio to station mode explicitly:

```sh
python3 dev/pi/probe.py stop-ap
```

If the remote wrapper was killed before its exit trap ran, its temporary iwd AP
profile may remain. Inspect that specific filename before cleanup; never remove
the station profile directory. Startup/cleanup failures are reported separately
so a cleanup error cannot hide the original error.

Do not overlap AP, station, or restart tests. `scan` deliberately refuses AP mode
until live AP scanning has been qualified on this driver.

## Restart observation

Run `python3 dev/pi/probe.py monitor` in one terminal, and
`python3 dev/pi/pi.py exec /etc/init.d/S40iwd restart` in another. The same adapter
rediscovers the name owner and device objects on each observation. It reports
Unavailable/Unknown during observed downtime and recovers without reconstruction.
The monitor runs for 30 seconds. The probe never modifies saved AutoConnect
settings; normal iwd auto-connection remains available.
