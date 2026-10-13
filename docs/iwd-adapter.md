# Direct iwd adapter

The Step 2 adapter lives in `crates/wifi-setupd/src/wifi/iwd`. Its typed zbus
bindings are private to the daemon. The reusable public API still contains no
iwd identifiers or types. `dev/iwd-probe` is a separate, development-only
workspace for real-Pi acceptance; the daemon entry point remains a scaffold
until the Step 3 actor/service is implemented.

## Authority and supported operations

The test Pi reports iwd **3.12**. Interface signatures and behavior were checked
against that release's `src/device.c`, `station.c`, `network.c`, `agent.c`,
`knownnetworks.c`, and `ap.c` in the Buildroot build tree. Upstream reference
links: [station](https://kernel.googlesource.com/pub/scm/network/wireless/iwd/+/master/doc/station-api.txt),
[network](https://kernel.googlesource.com/pub/scm/network/wireless/iwd/+/master/doc/network-api.txt),
[agent](https://kernel.googlesource.com/pub/scm/network/wireless/iwd/+/master/doc/agent-api.txt),
[AP](https://kernel.googlesource.com/pub/scm/network/wireless/iwd/+/master/doc/access-point-api.txt).
The checked 3.12 source takes precedence over moving upstream documentation.

Construction requires an injected zbus connection, exact interface name,
address reader, and bounded operation/poll durations. Construction, status, and
known-network reads do not change mode, power, saved AutoConnect, or association.
iwd can connect automatically to its own profiles. The future actor owns the
startup grace window and AP fallback policy.

| Operation | Behavior |
| --- | --- |
| Snapshot | Fresh ObjectManager view plus interface-scoped address verification |
| Scan | Station Scan, wait for Scanning=false, then ordered network metadata |
| Saved networks | SSID, security type, AutoConnect; never secret material |
| ConnectKnown | Ask iwd to use the matching saved profile |
| Connect | New credentials through a temporary, scoped agent; refuse existing profile |
| ReplaceCredentials | Explicit intent: Forget matching SSID/security, then submit new credentials |
| Forget | Delete only matching SSID/security through iwd; explicitly disconnect a matching active connection |
| Start/Stop AP | Switch Device.Mode, wait for the appropriate interface, call Start or StartProfile / Stop |
| Cancel/reconcile | Stop the accepted radio action and observe state before clearing pending work |

Submitted station secrets live only in the pending agent callback and its
D-Bus reply. The agent checks the sender's unique bus name and exact target
network path. A secret can be consumed once; cancellation, error, or dropping
the connection future clears the pending slot. Debug output redacts the complete
Credentials structure. This is scoped memory lifetime, not a guarantee that all
allocator/serialization buffers are cryptographically erased.

## Readiness, errors, and lifecycle

In iwd 3.12, Station.State=`connecting` includes network configuration. A
`connected` or `roaming` state plus an extant ConnectedNetwork and a usable
IPv4 address on the selected Wi-Fi interface produces Ready. Without that
address, report Associated. Loopback, unspecified, multicast, broadcast, and
IPv4 link-local addresses are excluded. Neither Ethernet nor Internet checks
can satisfy this probe. The current readiness reader is deliberately IPv4-only;
IPv6-only readiness needs address validity/tentative-state checks before support
is claimed. The test image has `EnableNetworkConfiguration=true`.

Operations target the unique owner obtained from the bus, not the reusable
well-known name. Every polling observation fetches a new ObjectManager snapshot
and verifies owner/device identity; nothing uses cached iwd object paths between
operations. Missing backend/device reports Unavailable/Unknown. Object removal
or owner replacement during an operation fails the operation; it is not replayed.
A later observation can discover the replacement. The adapter uses bounded
polling (250 ms during transitions), without a permanent background task or
cached signal subscription. The actor's long-running watch ownership is Step 3.

An accepted radio/profile mutation remains pending after failure, timeout, or future drop;
conflicting mutations are refused until explicit reconciliation. Stale operation
IDs cannot cancel newer work. Forget disconnects a matching active network and
enables radio cleanup only when that disconnect is needed; it leaves unrelated
connections alone. Cancellation cannot resurrect forgotten credentials.
Mode-setting, Forget, and its explicit disconnect retain pending replies across caller future
drop; cleanup waits for those non-cancellable calls to settle before claiming
the device is quiescent. Status reads have a three-second upper bound and report
Unavailable/Unknown on a backend timeout; they recover on later observations.

**iwd's generic `net.connman.iwd.Failed` is not proof of an incorrect password.**
It is mapped to a sanitized generic error. The new automatic-forget requirement
needs positive evidence of credential rejection; the basic iwd 3.12 Connect API
alone does not supply a distinct reason for every authentication failure.
Do not implement automatic credential deletion by treating every Failed, timeout,
or DHCP failure as AuthenticationFailed. Explicit user replacement/deletion is
fully separate and does not depend on this classification.

## Credential policy

Only iwd persists credentials. For user-requested same-SSID overwrite, old
credentials are deliberately discarded using iwd Forget before Connect. If the
new attempt fails, no old secret is restored. Unrelated profiles are preserved.
iwd may persist the new profile before application-level IP verification;
Connect failure therefore does not prove no profile was written. Check iwd's
known-network metadata and perform scoped cleanup in hardware tests.

No application profile files, secret backups, staging credential database, or
rollback store are introduced. Later HTTP/UI workflows must carry the user's
replacement intent explicitly. The connected UI will also expose listing and
deletion of old networks while the server keeps running (Step 4).

## Local and Pi evidence

The Pi's brcmfmac driver rejected the default iwd AP channel configuration with
`START_AP failed: -52`. An iwd AP profile with `[General] DisableHT=true` starts
successfully on channel 6 and supplies an AP-side IPv4 address. `AccessPointConfig`
supports either in-memory credentials or an existing iwd profile name; the
adapter only calls D-Bus methods and never reads or writes the profile. The dev
wrapper prepares and cleans a temporary iwd profile for the phone test. Production
profile/image configuration belongs in BuildrootImages. Phone association/DHCP
passed: the user reported `192.168.16.2`, independently matched to the active
Pi AP at `192.168.16.1/28` with one associated station. An AP-side address alone
would not establish this result.

Private-bus tests cover passive startup/auto-connect observation, address-gated
readiness, scans, agent callbacks and sender/path validation, AP/station mode
changes, known-profile connection/deletion, failed explicit replacement without
rollback, preservation of unrelated profiles, deadlines/reconciliation, and
owner/device replacement. Test address readers cannot inspect host interfaces.

Hardware observations are recorded above; remaining qualification is covered in
[the testing plan](testing-plan.md). Operator commands are in
`dev/iwd-probe/README.md`.
Driver WPA3 capability, real authentication failure behavior, and
persistence timing require hardware evidence and cannot be established by mocks.
