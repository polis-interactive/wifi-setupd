# Provisioning actor and service

Step 3 implements a single Kameo 0.22.2 actor, a production zbus exporter, and
the daemon executable. The API crate remains independent of Kameo and iwd.
HTTP endpoints and the phone portal are Step 4; the daemon has no web UI yet.

## Runtime configuration

```sh
wifi-setupd --interface wlan0 --ap-profile wifi-setupd
```

The interface and iwd AP profile SSID are explicit, non-secret arguments.
The profile must already exist in iwd's AP configuration; the daemon never reads
or writes credential files. For the tested Pi, the image-owned AP profile needs
`[General] DisableHT=true`. Provision that configuration in BuildrootImages;
the temporary development probe profile is removed after startup and is not a
production profile. The system bus must permit ownership and access to
`tv.polis.WifiSetup1` and the required iwd operations. Service installation/bus
policy and image packaging remain deployment work in Step 5.

`--startup-grace-seconds N` changes the default 20-second grace window.
`--no-auto` waits for an explicit D-Bus Start instead of opening a setup AP
automatically. `--help` runs without accessing D-Bus or hardware.

A startup with working station Wi-Fi publishes Idle/Ready without radio changes.
Otherwise, available iwd gets the grace window to auto-connect. Ethernet never
counts as Wi-Fi readiness. Before starting an AP, the worker scans and checks
readiness again, allowing auto-connect completed during that scan to win.
An existing active AP is adopted as AwaitingCredentials. The AP has no shutdown
timer. Explicit Stop disables automatic AP fallback until another Start or a
restart; Stop in Idle preserves any working Wi-Fi connection.

## Ownership and cancellation

`provisioner::Handle` exposes Start, Stop, status subscriptions, and internal
credential submission / saved-network connection methods for later HTTP code.
Submission is accepted only while AwaitingCredentials. Replacement intent is
explicit; neither messages nor public errors contain backend diagnostic text.
Credentials are temporary worker input; only iwd persists station credentials.

The actor owns workflow state, a monotonic operation ID, and a read-only watch
snapshot for consumers. One serialized adapter worker runs outside its mailbox.
Typed completion and verification messages carry the operation ID. Observation
messages carry an epoch so observations preceding a radio change cannot overwrite
its result. The actor never awaits network work in a command handler.

Default bounds are 90 seconds per workflow operation, 15 seconds per cleanup
attempt, and one second between observation/recovery ticks. These are operation
bounds, not AP lifetime limits. Dropping a timed-out/cancelled adapter future
is followed by `cancel_and_reconcile`. If reconciliation fails, state stays
Stopping, Start is rejected as Busy, and cleanup retries without admitting a
conflicting mutation. No claim of quiescence is made until cleanup succeeds.
Stop is idempotent and remains acceptable while the backend is unavailable.
A result arriving after Stop cannot publish provisioning success.

SIGINT/SIGTERM request Stop and allow up to 30 seconds for confirmed cleanup.
If cleanup remains uncertain, shutdown reports an error instead of claiming the
radio stopped. A process crash or forced termination cannot guarantee cleanup;
a later startup observes the actual iwd state.

## Connection and recovery

A submitted attempt stops the AP, switches to station mode, and performs a fresh
scan before resolving/connecting the selected network. It then verifies a fresh
Ready snapshot. This addresses the stale discovery ordering exposed by the Pi
probe; qualification of this actor-driven sequence on hardware is still pending.

Success publishes Idle/Ready/Available/None before one ProvisioningCompleted
signal. Repeated Start/Stop in that state does not emit another completion.
Failure reconciles and restores the AP while preserving the attempt's LastError.
A later accepted attempt clears that error. AP startup failure becomes Failed
only after cleanup has settled. Generic iwd failures never establish bad credentials.
For a saved-profile attempt with a **confirmed** Authentication error, the worker
forgets only that selected network before restoring provisioning. iwd 3.12's
current adapter does not yet provide reliable positive rejection evidence for
its generic Failed response or passive auto-connect failures; that hardware
classification gap remains explicit, and no profile is deleted on a guess.

Observations use fresh iwd ObjectManager snapshots, so service owner/device
changes do not reuse cached paths. The production adapter reconnects a closed
system-bus connection; injected test connections never fall back to the host bus.
The executable reattaches its D-Bus exporter after bus loss, retaining the actor.
The exporter subscribes before obtaining an actor revision checkpoint, discards
older updates, and never replays earlier completion signals to newly attached
clients. Individual property reads may span revisions; Status is coherent.

## Validation

Paused-time actor tests cover first boot/grace, saved auto-connect, readiness
finishing during scan, an AP surviving ten simulated minutes, repeated requests,
timeouts, cleanup retries, stale completion rejection, backend recovery, scoped
confirmed-authentication cleanup, and successful submission. Production-exporter
private-D-Bus tests cover wire properties, publication/completion ordering,
responsive cancellation, and exporter/client reconnection. Tests inject mocks
and private buses; they never change the laptop or Pi Wi-Fi.

Run the workspace tests, formatting, and strict Clippy commands in README.md.
The executable also cross-builds for AArch64 musl with the existing development
suite. No Step 3 test deploys to or changes the running Pi AP automatically.

Implementation follows Kameo's [Actor lifecycle](https://docs.rs/kameo/0.22.2/kameo/actor/trait.Actor.html)
and [typed Message handlers](https://docs.rs/kameo/0.22.2/kameo/message/trait.Message.html).
