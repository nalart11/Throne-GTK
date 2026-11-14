# Bridge contract, version 1

Rust owns the Store and connection state. Swift renders snapshots, never guesses
that an accepted connection command has completed. All C calls run off MainActor.

The UTF-8 C ABI returns owned NUL-terminated JSON strings. Each non-null result
must be released exactly once by `throne_macos_string_free`. Input strings are
borrowed only for the call. No callbacks cross the ABI. Handles are numeric,
validated by Rust; closed handles return errors rather than dereferencing freed
memory. Panics must not unwind across C. Call close once after stopping polling.

```
char *throne_macos_open(const char *options_json);
char *throne_macos_request(uint64_t handle, const char *request_json);
char *throne_macos_poll(uint64_t handle, uint32_t timeout_ms);
char *throne_macos_close(uint64_t handle);
void throne_macos_string_free(char *value);
```

Every response is `{ "ok": true, "value": ... }` or
`{ "ok": false, "error": "diagnostic" }`. Poll with no event returns null
as `value`. Poll events use `{ "kind": "changed" }` to request a snapshot;
additional event-specific fields may be present. Backend errors are surfaced,
not swallowed. Commands return accepted/results; engine events update snapshots.

Open options: `data_dir` (explicit directory; omitted means existing macOS
Application Support/throne-gtk), `core_path` (optional, default adjacent core).
Database filename/path must match current app paths. Tests always provide a
temporary `data_dir`. Open does not connect or change system configuration.

Snapshot request: `{ "op": "snapshot" }`. Value contains:
`groups`, `profiles` (all groups), `settings` (existing serialized Settings),
`status` (`kind`: disconnected/connecting/connected/failed, plus `profile_id`,
`name`, `error` where applicable), `selected_id`, `logs` (bounded strings),
`connections`, `traffic` (`down`, `up`, `total_down`, `total_up`), `auto_status`,
`testing`, `speed_stage`, `notice` (optional string).
`Settings` and `RouteRule` use existing serde field names. `Profile` and `Group`
do not derive serde in the existing libraries; bridge wire adapters expose their
existing public field names, including the nested `Profile.outbound` JSON object.

Requests (fields beside `op`):

| op | fields / result |
| --- | --- |
| select | id (negative group ID means auto-selection) |
| connect | optional id (otherwise current selection); VPN additionally requires acknowledge_vpn: true after explicit user confirmation |
| disconnect | — |
| settings_patch | patch: object of changed Settings fields; merge in Rust |
| group_save | group: existing Group object; id 0 inserts |
| group_delete | id |
| profile_save | profile: existing Profile object; id 0 inserts |
| profile_delete | ids: array |
| import_links | text, gid |
| import_throne | path |
| import_routes | path |
| subscription_refresh | gid |
| test_latency | ids: array (empty means selected group, supplied gid) |
| stop_test | — |
| speed_test | id |
| check_config | id |
| auto_recheck | — |
| auto_pin | name (empty removes pin) |
| logs_clear | — |
| export_config | id; returns generated configuration, not a new generator |

Errors from existing dependencies may retain their original diagnostic language;
all native interface controls and explanatory text are translated locally.
Settings language is persisted exclusively as the existing `language` field,
`ru`/`en`; Swift must not initialize or overwrite it with an OS/UserDefaults locale.

Initial lifecycle limitations: overlapping core operations are rejected, and
`stop_test` returns an explicit unsupported-operation error until the existing
engine's detached-worker cancellation is fixed after shared-file handoff. This
is not full GTK parity and must remain visible in acceptance notes.

Build harness remains isolated: no root workspace/lockfile edits, a separate
Cargo target directory and native output, executable basename `throne-gtk`.
