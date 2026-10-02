# Herdr Desktop Pet host integration

**Requires a patched Herdr host.** Stock Herdr 0.9.0 and 0.9.3 do not provide the `client.attached` plugin hook. The pet manifest's `min_herdr_version = "0.9.3"` is only the baseline for the supplied patch; it does **not** prove hook support. Use this patch on the official Herdr v0.9.3 source at commit `7b116c05bfda646af39d2524c54e70c751f57ee8`, or a future *actual* host release that explicitly advertises `client.attached`. Do not rely on an unknown-hook manifest warning being fatal: Herdr can link the plugin while its attach hook does nothing.

Pet **v0.1.4 includes the lifecycle settings**; the older 0.1.3 prebuilt does not. The manifest's `[[build]]` runs `bash scripts/install.sh` without `--source`, so managed installs (`herdr plugin install hanbong5938/herdr-desktop-pet`) download the prebuilt release pinned to the manifest version anonymously with `curl`, verify it, install it into the plugin directory's `dist/`, and fall back to a source build only if the prebuilt is unavailable or invalid. `herdr plugin link` does not run `[[build]]`; for a linked dev checkout, run `bash scripts/install.sh --source` yourself to build local changes, as shown below. `scripts/install.sh --prebuilt` installs only the pinned prebuilt, with no source fallback. All installation modes still require the patched Herdr host for client-attach auto-start. Do not substitute an unpatched Homebrew/stable Herdr binary for the patched binary.

## Build in separate checkouts

Prerequisites: macOS arm64 for the pet; Rust/Cargo, Bun, npm, Xcode command-line build tools and the tools needed by the two source trees. Build the Herdr host with the repository's normal locked Rust toolchain and native dependencies (including Zig 0.16.0 for vendored libghostty-vt); consult the official Herdr `README.md` and `AGENTS.md`. `just` is needed for its test/check recipes. For the full `just check` on macOS, the Herdr Windows cross target additionally needs `cargo install xwin --locked` and `just setup-windows-cross` (interactive SDK license acceptance; see Herdr `CONTRIBUTING.md`). Ordinary macOS builds do not need the Windows SDK.

From the **pet source** checkout, keeping its absolute path for linking:

```bash
PET_ROOT="$PWD"
HERDR_SOURCE="$(mktemp -d "${TMPDIR:-/tmp}/herdr-v0.9.3-patched.XXXXXX")"
git clone https://github.com/herdrdev/herdr.git "$HERDR_SOURCE"
cd "$HERDR_SOURCE"
git checkout 7b116c05bfda646af39d2524c54e70c751f57ee8
git rev-parse HEAD  # must be 7b116c05bfda646af39d2524c54e70c751f57ee8
git apply --check "$PET_ROOT/integrations/herdr/client-attached.patch"
git apply "$PET_ROOT/integrations/herdr/client-attached.patch"
just test
just check       # after setting up the Windows cross target as described above
cargo build --release --locked
```

Use `"$HERDR_SOURCE/target/release/herdr"` explicitly as `PATCHED_HERDR` below. Do **not** copy it over an installed Herdr, replace an existing server, or change your update channel implicitly. Start and inspect a separate disposable Herdr session with the patched binary (and avoid inherited `HERDR_SOCKET_PATH` / `HERDR_CLIENT_SOCKET_PATH` from another session; see the host's `AGENTS.md`). API/schema checks must use the *same patched binary* and target session, not a stable binary already on `PATH`.

Back in the pet checkout, build its packaged app explicitly from local source, then link the source directory with the patched host. Herdr `plugin link` does not execute the manifest build command:

```bash
cd "$PET_ROOT"
./scripts/install.sh --source
PATCHED_HERDR="$HERDR_SOURCE/target/release/herdr"
HOST_PROFILE="$(mktemp -d /tmp/herdr-pet.XXXXXX)"
export XDG_CONFIG_HOME="$HOST_PROFILE/config" XDG_STATE_HOME="$HOST_PROFILE/state"
export XDG_DATA_HOME="$HOST_PROFILE/data" XDG_CACHE_HOME="$HOST_PROFILE/cache"
unset HERDR_ENV HERDR_SOCKET_PATH HERDR_CLIENT_SOCKET_PATH HERDR_CONFIG_PATH
unset HERDR_PLUGIN_CONFIG_DIR HERDR_PLUGIN_STATE_DIR
"$PATCHED_HERDR" server &
# Wait for this server's "api socket:" message before the commands below.
"$PATCHED_HERDR" plugin link "$PET_ROOT"
"$PATCHED_HERDR" plugin list
"$PATCHED_HERDR" api schema --json
```

Inspect the `plugin list`/`plugin.link` warnings for `unknown event 'client.attached'`; no such warning should appear. Inspect the printed API schema for a `client.attached` subscription and `client_attached` event. An explicit raw API probe on the patched session is `{"id":"attach_probe","method":"events.subscribe","params":{"subscriptions":[{"type":"client.attached"}]}}` sent to its API socket: require a successful subscription acknowledgment, attach a **shell or terminal** client to the same session, and observe an event with `event: "client_attached"`, `data.type: "client_attached"`, server-local numeric `data.client_id`, and `data.client_kind: "shell"` or `"terminal"`. A plain API request, failed client handshake, or handoff must not emit attach. A successful link or version match alone is **not** this capability check.

## Lifecycle behavior

The manifest keeps the server-ready `[[startup]]` ensure and adds a distinct `client.attached` ensure for each successful client registration. These are asynchronous plugin hooks, filtered by the host plugin enabled state and macOS platform; ensure is idempotent and does not force an already running/hidden pet visible. Host `plugin disable` or `unlink` remains separate from pet lifecycle preferences. Client detach is **not** server death. When `exit_with_herdr` is ON, the pet watches healthy server connections and exits only after the last healthy server has been absent for 30 seconds; turning it OFF retains independent operation. Fresh settings default both `auto_start` and `exit_with_herdr` ON; upgraded profiles preserve the old independent exit behavior (OFF), and legacy `enabled=false` maps to `auto_start=OFF`. Manual start is independent of auto-start, and stop/Quit does not change the preference.

The Herdr `settings` plugin action launches the native settings window without starting the pet daemon, but Herdr actions require an available enabled host. For offline or host-disabled use, call the **packaged native binary directly**; these commands do not require Herdr and settings get/set do not launch the pet:

```bash
PET="$PET_ROOT/dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet"
"$PET" settings
"$PET" settings get
"$PET" settings set auto_start off
"$PET" settings set exit_with_herdr on
```

Pet v0.1.4 supplies the pet-side lifecycle changes, not the missing host hook. Client-attach auto-start still requires the supplied patch on official Herdr v0.9.3. Only advertise support for an unpatched host once a real host release explicitly ships `client.attached`; stock v0.9.3 does not.
