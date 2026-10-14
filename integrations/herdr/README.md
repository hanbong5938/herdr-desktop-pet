# Desktop Pet — Herdr integration

This repository owns the Herdr adapter, plugin manifest, thin consumer wrappers, host patch and OMP outcomes extension—not the native app/UI/characters/build. [Desktop Pet core installation and usage](https://github.com/hanbong5938/desktop-pet) · [exact profiles/discovery](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.md#profiles-and-installation-discovery) · [legacy migration entrance](https://github.com/hanbong5938/desktop-pet/blob/main/docs/migrations/desktop-pet.md).

**Publication boundary:** actual [Core v0.4.1](https://github.com/hanbong5938/desktop-pet/releases/tag/v0.4.1) and its retained consumer/Brew evidence remain valid historical records. Published Herdr v0.4.0 retained an erroneous generated-native `dist` source closure; preserve its immutable tag/body/assets. The [corrected Herdr v0.4.1 candidate](../../docs/releases/v0.4.1.md) requires exact actual Core v0.4.2 (protocol 4 / observation capability 2 / integration API 1), not yet public in this record. Bridge remains v0.3.7 with corrected immutable pins; Tern v0.2.0 is unchanged. No current completed migration/installation matrix or future release metadata is claimed.

**Consumer source cutover:** the corrected manifest/wrappers/adapter belong to this checkout; obsolete common source/assets/tooling and `dist` product closure do not. Whole working/index ownership is guarded before overlay, and archived Git trees are guarded before tagging/publication. The consumer pin still requires actual anonymously verified Core v0.4.2 installer/checksum and reviewed release API metadata. Existing `core-release.json` / `release-evidence/` are retained actual v0.4.1 historical evidence until the parent materializes real v0.4.2; the new consumer rejects those old metadata. Host patches, all 21 actions, hook and OMP authority remain integration-owned.

Core v0.4.2 and corrected Herdr v0.4.1 publication, packaged acceptance and the final matrix are separate parent-owned gates. Source/documentation changes establish neither real future release authority nor a completed public installation. See the [publication sequence](../../docs/releases/publishing.md#corrected-v041-parent-publication-sequence); no tag is moved and no bridge ownership gate is weakened.

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

`herdr plugin install` reruns integration update only. `plugin uninstall/unlink` does not delete the shared native app/profile/characters. Native updates never reinstall this plugin, except the explicitly consented **one-time managed legacy migration retirement** using the same registry and the corrected integration-only immutable v0.4.1 ref after real publication. Auto retains the pet/other provider when Herdr disappears. Herdr-only disconnect/disable lifecycle remains adapter-owned. Tern's separate plugin neither installs nor starts the native app.

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

## Optional guarded Plugin Manager

The embedded Manager window belongs to the Herdr adapter; the neutral core supplies the shared profile owner, preference transaction and lifecycle action surface, not Herdr sockets or mutation commands. There is no independent Manager app or helper to install. The selected profile starts with `plugin_manager_installed` false. **Install** enables availability, **Remove** disables it after explicit confirmation, and **Open** starts discovery on the selected socket. Install/Remove do not mutate a plugin or initialize unused operation jobs. Remove refuses a plugin operation whose private lifetime lease is still held in any process. Closing the window, hiding the Manager or quitting Pet does not cancel an already submitted host CLI operation.

Host mutation support additionally requires [plugin-manager.patch](plugin-manager.patch), applied **after** `client-attached.patch` on the pinned isolated host checkout below. Stock v0.9.3 has no Manager mutation support. This patch contains all 18 affected files, including `plugin_paths.rs` and the generated `docs/next/api/herdr-api.schema.json`; consumers do not manually regenerate that schema. The preserved patch is incoming source, not evidence that this checkout built a host or replaced an installed/public host.

Discovery authenticates the actual selected Unix socket, same-UID peer PID and canonical peer executable, and binds their identities. Stock or older protocol-2 hosts may provide ordinary read-only `plugin.list` inventory where available. All five mutations require capabilities with `product_id: "herdr-plugin-manager-host"`, `protocol: 2`, `manager_safety_protocol: 1`, `durable_result_protocol: 1` and `admission_fd_protocol: 1`; the live context must also advertise protocol 2 and safety protocol 1. A malformed safety-1 inventory never downgrades to legacy inventory. There is no stock-CLI, older-protocol or alternate-host mutation fallback.

The host exposes `herdr plugin manager capabilities --json`, `context --socket ABS --json` and internal `apply --request ABS_JSON --admission-fd N`. Do not invoke `apply` by hand. Pet locks the owned private per-operation `operation.lock`, syncs the request and every journal ancestor before publishing its active locator or spawning the authenticated host image, and hands that same locked descriptor to the CLI. The CLI verifies the exact inode and sets close-on-exec immediately so build/git descendants cannot prolong its lease. Requests and durable results use compact newline-free JSON with inclusive entire-file limits of 1 MiB and 128 KiB. Pet observes a bounded, fsynced, atomically published `result.json` only after the lease ends. Output, a parent process exit or a missing result never prove success. Ended Unknown operations retain their exact original ID for explicit review and acknowledgement; acknowledgement never replays a mutation, and cancellation of review never clears the block on later operations.

Fresh displayed metadata and original registration come from one read-only inventory snapshot; both are rechecked before mutation. Enable/Disable apply to valid nonprotected registrations, including managed or no-source entries. Disable clears only the matching agent view, not pane ownership or terminals. Unlink applies only to local links and retains files. Update/Uninstall require a complete managed GitHub checkout, retain config/state, and Update preserves disabled state and the recorded ref. Successful Unlink/Uninstall release matching pane ownership without closing terminals.

All five actions protect `desktop-pet`, the actual current executable/enclosing bundle discovered internally, registered Pet roots, managed Pet checkouts, physical registry/lock/temp aliases and ancestor/descendant overlaps. Caller-provided roots cannot replace current-image protection. The host authenticates the actual RPC stream's UID, original PID and executable and rechecks immutable canonical config/state roots before lease validation or filesystem mutation. Ordinary install/uninstall and Manager filesystem work share `.plugin-install.lock`; ordinary link/unlink/enable/disable use nonblocking `.manager.lock` admission and return `plugin_manager_busy` during a Manager transaction. Manager-only server mutations require the live CLI's private lease token.

The adapter uses the core-injected shared `ProfileStore`, verifies its exact config/state pairing, and reads only committed selected-profile availability without importing legacy preferences, quarantining malformed data or using another profile's defaults. Its real owned availability admission FD remains held across the common atomic preference transaction. Common Prepare/migration/retirement admission must approve the final offered mutation; the adapter cannot substitute a synthetic permission guard. Live helpers must verify both selected config and state, and offline preference writes require the common config-scoped owner through save, including when another state directory was selected. A late or cancelled window request cannot revive an old operation after retirement.

Relative config input resolves against the real process CWD before private-owner, leaf-symlink and inode checks. State, arbitrary protected roots, managed checkout paths and request paths remain absolute-only; ambiguous old relative `managed_path` is rejected rather than rebased. Config ancestor aliases retain physical admission/lifetime exclusion and cannot change a recorded config/state pairing. The immutable original host config root is pinned before registry creation/locking and checked again at precommit; an outside-config stow target remains supported.

The host's installer and backup validation require one exact layout: canonical checkout plus a grammar-validated, lexically joined recorded subdirectory, never a redirected canonicalized plugin root. Nested paths, scoped names, spaces and Unicode remain supported; traversal, NUL, backslash, colon and symlinks remain rejected. Root/manifest checks run before and after build, before network/swap, after final swap, at server admission, in-lock and against the preserved old backup. Update validates old-registration CAS and verified backup root/manifest identity. A definitive post-swap failure restores the old checkout through rollback, or returns Partial with its backup retained if inventory is inconsistent.

Registry durability syncs existing and new ancestor chains on every operation without a global cache, full-syncs the temporary file, renames it and full-syncs the same pinned target-parent FD before completion. Precommit errors are definite failure with an unchanged registry; post-rename uncertainty retains `plugin_manager_registry_uncertain` and Unknown rather than claiming a failure/success. Host memory, pane ownership and agent views change only after durable success. Unknown Update retains the new checkout and old backup; Unknown Uninstall retains checkout and user data.

Availability helpers must use typed absence checks: only connect-time `NotFound` or `ConnectionRefused` establishes absence, never a connected timeout/empty EOF. Framing/parse/permission failures stay failures after socket disappearance. An authenticated stopping response remains a live owner until ownership is released. One bounded readiness/stopping deadline applies before the counted Set write; every result after Set entry is returned directly without offline fallback or replay, and partial transmission retains exact-ID uncertainty. Matching startup owners may be awaited, but a live wrong-state owner must fail without overwrite.

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
# Optional guarded Plugin Manager support, after the client-attach patch:
git apply --check "$INTEGRATION_ROOT/integrations/herdr/plugin-manager.patch"
git apply "$INTEGRATION_ROOT/integrations/herdr/plugin-manager.patch"
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

Shipped legacy 0.3.6 must perform **A → real legacy bridge 0.3.7**, wait for A terminal/lease/reservation release, then separately consent to **B → neutral 0.4.2**, pinned to actual new public Core and corrected immutable Herdr 0.4.1. Same-profile image/readiness proof, user Stop, Unknown/no manager replay, exact rollback and owner-only legacy retirement are required; runtime-ready without retired old install is not complete. Protocol-2 beta uses its retained matching helper, actual stop and **pinned frozen 0.3.6** prebuilt before A→B, not the current rolling stable formula. Tern Configure keeps its old two paths and requires actual reconnect; the app never edits its KV. See the [full migration contract](https://github.com/hanbong5938/desktop-pet/blob/main/docs/migrations/desktop-pet.md).

All actual installation/migration/recovery smoke must run in a **disposable macOS account or VM**, not merely changed HOME/XDG: global gates use the real account home. The user's real plugin, agents and worktrees are not disposable fixtures. No neutral/bridge runtime or public cutover success is asserted here; historical releases remain unchanged.
