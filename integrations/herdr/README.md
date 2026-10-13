# Desktop Pet — Herdr integration

This repository owns the Herdr adapter, plugin manifest, thin consumer wrappers, host patch and OMP outcomes extension—not the native app/UI/characters/build. [Desktop Pet core installation and usage](https://github.com/hanbong5938/desktop-pet) · [exact profiles/discovery](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.md#profiles-and-installation-discovery) · [legacy migration entrance](https://github.com/hanbong5938/desktop-pet/blob/main/docs/migrations/desktop-pet.md).

**Publication boundary:** legacy [Herdr v0.3.6](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/v0.3.6) is shipped with protocol 3 / observation capability 1. Independent Desktop Pet v0.4.0 (protocol 4 / observation capability 2 / integration API 1) and the legacy bridge v0.3.7 are reviewed candidates, not verified public installations. Commands below describe the neutral cutover; do not treat notes, a source version or a tag as publication proof.

**Consumer source cutover:** the integration-only manifest/wrappers/adapter now live in this checkout; obsolete common app source/assets/tooling were transferred to core and removed here. This is not public installation proof: the consumer pin must still be materialized from the actual anonymously verified public core installer/checksum and reviewed release API metadata. Public release/formula/integration publication and host acceptance are separate owner gates.

Public core documentation links below are destination paths for common ownership; core v0.4.0, bridge v0.3.7 and the integration-only tags are not yet published. Packaged acceptance and final publication are separate parent-owned gates. No source/documentation change establishes a public installation or a completed final matrix.

## Consumer installation

After actual neutral release/formula publication:

```sh
brew install hanbong5938/tap/desktop-pet
herdr plugin install hanbong5938/herdr-desktop-pet
herdr plugin action invoke start --plugin desktop-pet
```

The integration installer reuses a verified compatible installation with `integration_api:1`; only absent native installation permits fetching the pinned, digest-verified core release installer into the shared standalone location. It does not infer core version from this plugin version, downgrade/replace an incompatible installation, compile core here or hide download failure behind source fallback. Native source/prebuilt installation belongs exclusively to the core checkout (`--source`, not `--local-build`). The wrapper’s discovery order is explicit absolute path → validated shared locator → PATH `desktop-pet` → actual Brew prefix → account-home `Applications/DesktopPet.app`; invalid explicit path/locator fails closed. A locator alone is not capability/signature/origin proof, and a different running image/profile is shown rather than silently launched.

Herdr `HERDR_PLUGIN_CONFIG_DIR` / `HERDR_PLUGIN_STATE_DIR` precedence is interpreted **only by this wrapper**, then passed as explicit native paths. Core defaults instead use neutral environment/default-profile record/XDG roots. Select and back up both actual directories; never move/merge legacy, isolated Tern, beta or custom profiles. Backend socket provenance becomes adapter-owned `--herdr-socket ABS`; it is not the pet’s private profile control socket. `status` reports the actual daemon executable/profile/version, not just the installed CLI. Manual local snapshots (Herdr 0.9.0+) need no plugin/patch; replies separately need server `agent.prompt`, and its ACK is not model completion.

Profile selection is per-directory: injected `HERDR_PLUGIN_CONFIG_DIR` / `HERDR_PLUGIN_STATE_DIR` override explicit `--config-dir` / `--state-dir`, which override `${XDG_CONFIG_HOME:-$HOME/.config}/herdr/plugins/config/desktop-pet` and `${XDG_STATE_HOME:-$HOME/.local/state}/herdr/plugins/desktop-pet`. The consumer passes both resolved absolute paths explicitly. These Herdr defaults are not the neutral core defaults. To share an existing neutral/custom profile, select its actual two paths; unset Herdr injection first when using explicit paths outside a host. Do not substitute a different HOME to bypass account-home installation ownership.

An integration-owned explicit invocation with an installed compatible core app is:

```sh
bash scripts/run-native.sh status \
  --executable /absolute/path/DesktopPet.app/Contents/MacOS/desktop-pet \
  --config-dir /absolute/config/profile --state-dir /absolute/state/profile \
  --herdr-socket /absolute/herdr/api.sock
```

`--executable` (or `DESKTOP_PET_EXECUTABLE`, not both) selects the exact app image; the shared locator is `<actual-account-home>/.local/share/desktop-pet/installation.json`. Neither a locator nor a CLI version alone proves installation ownership or runtime compatibility. Legacy `--socket` is rejected, not retained as an alias. Run/install wrappers never copy a product build or assets into the integration checkout.

`herdr plugin install` reruns integration update only. `plugin uninstall/unlink` does not delete the shared native app/profile/characters. Native updates never reinstall this plugin, except the explicitly consented **one-time managed legacy migration retirement** using the same registry and integration-only immutable v0.4.0 ref. Auto retains the pet/other provider when Herdr disappears. Herdr-only disconnect/disable lifecycle remains adapter-owned. Tern's separate plugin neither installs nor starts the native app.

## Retained 21 Herdr actions

Invoke `herdr plugin action invoke ACTION --plugin desktop-pet`. These existing IDs remain thin wrapper calls into the selected core executable/profile; installation/link is not an action-driven native build.

| Action ID | Retained meaning |
| --- | --- |
| `settings` | Open the selected profile's native settings UI. |
| `start` | Explicitly start the pet in the selected profile. |
| `ensure` | Idempotently ensure the selected instance, preserving saved provider and hidden state. |
| `stop` | Stop the selected profile's pet; later eligible hooks may relaunch if `auto_start` remains on. |
| `status` | Report the actual selected profile's running image, profile and version. |
| `toggle` | Toggle character visibility, retaining an enabled bubble. |
| `show` | Show the character and reattach the visible bubble. |
| `hide` | Hide the character, retaining a visible bubble standalone. |
| `passthrough` | Toggle full-window click passthrough for character and bubble. |
| `alpha_passthrough` | Toggle alpha click passthrough for the character only. |
| `show_bubble` | Show the bubble even without the character. |
| `hide_bubble` | Hide the bubble without hiding the character. |
| `bubble_above` | Place the bubble above the character when attached. |
| `bubble_below` | Place the bubble below the character when attached. |
| `bubble_left` | Place the bubble left of the character when attached. |
| `bubble_right` | Place the bubble right of the character when attached. |
| `bubble_auto` | Choose automatic attached bubble placement. |
| `reset` | Reset character and standalone bubble positions, retaining visibility. |
| `bigger` | Increase character size. |
| `smaller` | Decrease character size. |
| `restart` | Restart the selected profile's pet. |

Bubble placement does not move a standalone bubble; it takes effect on reattachment. Common presentation/settings detail belongs to core [usage](https://github.com/hanbong5938/desktop-pet/blob/main/docs/usage.md) and [CLI](https://github.com/hanbong5938/desktop-pet/blob/main/docs/cli.md), not a duplicate native source manual here.

## Session close in current source

**Current-source behavior, not a claim about published/prebuilt apps:** the bubble context menu's **Close Session** closes the corresponding **local Herdr pane**, not the bubble window. A card-header menu targets that card; a bubble-background menu targets the retained selected session, including in compact mode. It never falls back to the first row. Character-menu **Hide Bubble** and the `hide_bubble` action remain visibility-only controls.

Earlier published builds' **Close Conversation** was a visibility-only action; the source behavior above does not retroactively change that behavior or establish close support in a released host.

The app captures the session identity when opening the menu, then checks the canonical local endpoint, a fresh `session.snapshot`, the exact terminal/pane/workspace mapping, source generation and observation scope before mutation. Remote Herdr and Tern sessions are not close targets. Offline, stale, ambiguous or read-only targets fail safely instead of closing a replacement or another session. Closure leaves bubble/character visibility and placement unchanged; authoritative host snapshots reconcile the rows rather than optimistic local deletion.

The ordinary host request is `pane.close` with `{"pane_id":"<resolved pane ID>"}`; acknowledged success requires `result.type == "ok"`. Herdr may close a tab or workspace when its last pane is closed. If the host returns `confirmation_required` for a worktree group, the app offers a separate **Cancel-default confirmation** showing the workspace and pane counts. Unmodified Return/keypad Enter and Escape cancel this confirmation rather than approve it. Only explicit confirmation, followed by fresh validation that the exact group scope is unchanged, permits `workspace.close` with `{"workspace_id":"<captured workspace ID>","close_group":true}`. Cancellation, missing/ambiguous group metadata or changed membership does not escalate the pane request. This closes host workspaces/panes, not worktree files.

Snapshot observation, reply delivery and close are separate host capabilities: the documented Herdr 0.9.0+ snapshot support does **not** establish close support, and close does not depend on `agent.prompt`. The inspected v0.9.3-based host source exposes `pane.close` without `client-attached.patch`; that patch is only for optional attach auto-start. The **minimum released Herdr version supporting close is unverified**. Unsupported servers report a visible failure without an alternate destructive operation. A lost or malformed acknowledgment after an attempted write is reported as unknown delivery, never retried automatically or presented as definite failure/success.

**Source-only verification:** production AppKit group-confirmation smoke used a private owned Unix-socket fixture. Cancel and the native default cell sent no `workspace.close` and preserved the other selected session's draft and bubble visibility; explicit native approval fetched a fresh snapshot and sent exactly one `workspace.close` with `{"close_group":true,"workspace_id":"workspace-A"}`. Keyboard cancellation mappings are regression-tested, but runtime Return/keypad Enter/Escape delivery was not verified in the inactive/non-key isolated app. No live Herdr host pane was closed.

## Optional client-attach auto-start

Stock Herdr 0.9.0/0.9.3 do not provide `client.attached`. The manifest minimum is not hook proof; optional attach auto-start requires official Herdr 0.9.3 built with this repository’s [client-attached.patch](client-attached.patch), or an actual host release advertising that hook. Plugin installation never patches Herdr. Linking/enabling a running host does not itself start the pet: invoke start once or await a later eligible shell/terminal attach. `ensure` is idempotent, respects saved provider/profile and never reveals a hidden pet. Inspect/reject unknown hook warnings rather than treating a successful link as host compatibility.

## Build in separate checkouts

Prerequisites: macOS arm64 for the core; Rust/Cargo, Bun, npm, Xcode command-line build tools and the tools needed by the independent core and host source trees. Build the Herdr host with the repository's normal locked Rust toolchain and native dependencies (including Zig 0.16.0 for vendored libghostty-vt); consult the official Herdr `README.md` and `AGENTS.md`. `just` is needed for its test/check recipes. For the full `just check` on macOS, the Herdr Windows cross target additionally needs `cargo install xwin --locked` and `just setup-windows-cross` (interactive SDK license acceptance; see Herdr `CONTRIBUTING.md`). Ordinary macOS builds do not need the Windows SDK.

From the **Herdr integration** checkout, keeping its absolute path for linking (the independent core is a separate checkout):

```bash
INTEGRATION_ROOT="$PWD"
HERDR_SOURCE="$(mktemp -d "${TMPDIR:-/tmp}/herdr-v0.9.3-patched.XXXXXX")"
git clone https://github.com/herdrdev/herdr.git "$HERDR_SOURCE"
cd "$HERDR_SOURCE"
git checkout 7b116c05bfda646af39d2524c54e70c751f57ee8
git rev-parse HEAD  # must be 7b116c05bfda646af39d2524c54e70c751f57ee8
git apply --check "$INTEGRATION_ROOT/integrations/herdr/client-attached.patch"
git apply "$INTEGRATION_ROOT/integrations/herdr/client-attached.patch"
just test
just check       # after setting up the Windows cross target as described above
cargo build --release --locked
```

Use `"$HERDR_SOURCE/target/release/herdr"` explicitly as `PATCHED_HERDR` below. Do **not** copy it over an installed Herdr, replace an existing server, or change your update channel implicitly. Start and inspect a separate disposable Herdr session with the patched binary (and avoid inherited `HERDR_SOCKET_PATH` / `HERDR_CLIENT_SOCKET_PATH` from another session; see the host's `AGENTS.md`). API/schema checks must use the *same patched binary* and target session, not a stable binary already on `PATH`.

Install or explicitly build the core first in its own checkout with `bash scripts/install.sh --source`. Then return to this integration checkout and link it with the patched host. Herdr `plugin link` does not execute a build command:

```bash
cd "$INTEGRATION_ROOT"
PATCHED_HERDR="$HERDR_SOURCE/target/release/herdr"
HOST_PROFILE="$(mktemp -d /tmp/herdr-pet.XXXXXX)"
export XDG_CONFIG_HOME="$HOST_PROFILE/config" XDG_STATE_HOME="$HOST_PROFILE/state"
export XDG_DATA_HOME="$HOST_PROFILE/data" XDG_CACHE_HOME="$HOST_PROFILE/cache"
unset HERDR_ENV HERDR_SOCKET_PATH HERDR_CLIENT_SOCKET_PATH HERDR_CONFIG_PATH
unset HERDR_PLUGIN_CONFIG_DIR HERDR_PLUGIN_STATE_DIR
"$PATCHED_HERDR" server &
# Wait for this server's "api socket:" message before the commands below.
"$PATCHED_HERDR" plugin link "$INTEGRATION_ROOT"
"$PATCHED_HERDR" plugin list
"$PATCHED_HERDR" api schema --json
```

Inspect the `plugin list`/`plugin.link` warnings for `unknown event 'client.attached'`; no such warning should appear. Inspect the printed API schema for a `client.attached` subscription and `client_attached` event. An explicit raw API probe on the patched session is `{"id":"attach_probe","method":"events.subscribe","params":{"subscriptions":[{"type":"client.attached"}]}}` sent to its API socket: require a successful subscription acknowledgment, attach a **shell or terminal** client to the same session, and observe an event with `event: "client_attached"`, `data.type: "client_attached"`, server-local numeric `data.client_id`, and `data.client_kind: "shell"` or `"terminal"`. A plain API request, failed client handshake, or handoff must not emit attach. A successful link or version match alone is **not** this capability check.

## Adapter-owned lifecycle

`auto_start` controls Herdr server/client hooks, never Tern autostart. `exit_with_herdr` and endpoint data remain Herdr-owned keys in `lifecycle.json`; common and adapter keys persist atomically without dropping each other. In Herdr-only mode, enabled exit policy waits 30 seconds after loss of the last healthy server; turning it off retains independent operation. Fresh Herdr policy defaults on; a missing exit key in an existing profile retains the historical off migration. Client detach is not server death. Initially absent plugin membership is standalone observation, not disable; confirmed disable/unlink of every registered local endpoint triggers the selected Herdr-only policy. Auto/other provider/zero-host runs do not acquire Herdr startup/host-loss requirements. Keep `auto_start` off if later eligible hooks must not relaunch after Stop.

The first common lifecycle write initializes registered owners in the same atomic transaction: starting/ensuring a fresh full-feature app or first setting `auto_start` retains Herdr's enabled fresh default. Read-only lifecycle queries never materialize that default; existing owner-less lifecycle files keep the disabled missing-key policy, and Tern-only or zero-adapter apps do not write Herdr keys.

The boundary is file existence, not whether a key happens to be absent: fresh absent file → `exit_with_herdr: true`; existing file with the key missing → false. Explicit true/false always wins. Lifecycle writes preserve unrelated/unknown fields and other owners' keys; neither reads nor profile selection silently rewrite an existing policy.

Remote Herdr observation is read-only, configured via selected saved-machine profiles and interactive authentication in the actual Herdr terminal. It needs a compatible local forwarding CLI/remote server, not a remote pet install. Local-looking paths never upgrade remote mutation authority. Use core [usage](https://github.com/hanbong5938/desktop-pet/blob/main/docs/usage.md) and [CLI](https://github.com/hanbong5938/desktop-pet/blob/main/docs/cli.md) for generic controls and status.

## OMP outcome setup

The extension remains owned here at [`../omp/herdr-pet-outcomes.ts`](../omp/herdr-pet-outcomes.ts). Load it **alongside**, never instead of or by modifying, Herdr’s managed OMP state extension:

```sh
HERDR_INTEGRATION=/absolute/path/to/current/herdr-desktop-pet-checkout
omp --extension "$HERDR_INTEGRATION/integrations/omp/herdr-pet-outcomes.ts"
```

Use OMP with the `agent_end.willContinue` contract (the historical producer/consumer evidence used OMP 18.2.10). Preserve `omp:v1` / `pet_outcome_*` wire authority; acceptance/idle is not synthetic completion. Character-authoring packs/catalog and rig ABI remain core/catalog concerns, not host protocol revisions.

## Migration and acceptance

Shipped legacy 0.3.6 must perform **A → real legacy bridge 0.3.7**, wait for A terminal/lease/reservation release, then separately consent to **B → neutral 0.4.0**. Same-profile image/readiness proof, user Stop, Unknown/no manager replay, exact rollback and owner-only legacy retirement are required; runtime-ready without retired old install is not complete. Protocol-2 beta uses its retained matching helper, actual stop and **pinned frozen 0.3.6** prebuilt before A→B, not the current rolling stable formula. Tern Configure keeps its old two paths and requires actual reconnect; the app never edits its KV. See the [full migration contract](https://github.com/hanbong5938/desktop-pet/blob/main/docs/migrations/desktop-pet.md).

All actual installation/migration/recovery smoke must run in a **disposable macOS account or VM**, not merely changed HOME/XDG: global gates use the real account home. The user's real plugin, agents and worktrees are not disposable fixtures. No neutral/bridge runtime or public cutover success is asserted here; historical releases remain unchanged.
