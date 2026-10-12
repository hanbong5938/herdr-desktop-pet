# Herdr Desktop Pet

A native macOS companion for [Herdr](https://herdr.dev). Rubelia reacts to touch, shows observed agent sessions on your desktop, and lets you reply to a selected local session.

**English** · [한국어](readme.ko.md) · [简体中文](readme.zh-Hans.md) · [日本語](readme.ja.md) · [Version documentation](docs/releases/README.md) · [Releases](https://github.com/hanbong5938/herdr-desktop-pet/releases)

<img src="assets/rubelia-thumbnail.png" alt="Rubelia, the default desktop companion" width="220">

[Browse the optional character and outfit gallery](https://hanbong5938.github.io/herdr-characters/) (opens in English; choose KO for Korean) · [Character pack source and licenses](https://github.com/hanbong5938/herdr-characters). **Only Rubelia (`default@0`) is bundled.**

**v0.3.5 is this checkout's stable source/default-prebuilt target**, not proof that its binaries, installer or Homebrew formula are published. Check actual [Release assets](https://github.com/hanbong5938/herdr-desktop-pet/releases) before prebuilt installation; see the [bilingual v0.3.5 note](docs/releases/v0.3.5.md). Chaerin is an optional character pack, not a replacement for bundled Rubelia.

## Features

- A movable, resizable desktop character with touch and petting reactions, an independent status bubble, and menu-bar controls.
- Observed Herdr session cards with search, filters, ordering, and distinct local/remote source labels; remote observation is optional and read-only.
- One-line replies to selected **local** agent sessions, with IME-aware input and native status feedback.
- Customizable character, bubble, dialogue, and visibility settings; optional verified character packs and an official catalog browser.
- Herdr lifecycle actions, guarded linked-worktree removal, and explicit installation-aware app updates.

## Requirements

- **Apple Silicon Mac with macOS 13 or later.** Intel Macs, Windows, and Linux are not supported native targets.
- For local observation, use an existing local Herdr server (0.9.0 or later for snapshots); no plugin or host patch is needed. Replies separately require that server's `agent.prompt`. **Only optional client-attach auto-start** requires official Herdr 0.9.3 source built with the supplied [`client.attached` patch](integrations/herdr/client-attached.patch), or a future actual host release advertising that hook. Stock Herdr 0.9.0 and 0.9.3 lack it; the manifest version floor does not supply the hook. Follow the [host integration guide](integrations/herdr/README.md) and reject unknown-hook warnings in that managed setup.
- Source builds require Rust/Cargo, Bun, Node.js/npm, and Xcode Command Line Tools; a prebuilt needs no source toolchain. This app is **ad-hoc signed, not Developer ID signed or notarized**; macOS Gatekeeper may require explicit local approval. Signature checks do not imply notarization.

## Install with Homebrew: first connection

The [personal tap](https://github.com/hanbong5938/homebrew-tap) and non-draft v0.3.5 arm64 archive/checksums are publicly available; check the actual [Release assets](https://github.com/hanbong5938/herdr-desktop-pet/releases) and formula before installing. On the Mac displaying the pet, with an existing local Herdr server running:

```sh
brew install hanbong5938/tap/herdr-desktop-pet
herdr-desktop-pet --version
herdr-desktop-pet start
herdr-desktop-pet status
```

The native `start` is manual: it does not require `herdr plugin install`, `plugin link` or a patched host. Expand the bubble to see local session cards from that server (Herdr 0.9.0+ for snapshots); a live agent session is needed for a live card. Sending a reply additionally requires that server's `agent.prompt`; submission ACK is not completion. If cards do not appear, check the actual running Herdr CLI/server and `HERDR_SOCKET_PATH` / `HERDR_CLIENT_SOCKET_PATH`. If its backend socket differs from the pet's environment/default, use the **actual socket path printed by that server**, not a guessed path:

```sh
herdr-desktop-pet --socket /actual/path/to/herdr-api.sock start
herdr-desktop-pet status
```

Replace `/actual/path/to/herdr-api.sock` with the verified Herdr **backend** socket; `--socket` is not the pet's private control socket. `start` reuses an already-running daemon, so stop it before changing the selected backend; check `status` for the actual daemon version/executable, not just `--version`. See [connection troubleshooting](docs/usage.md#configuration-and-troubleshooting).

Before upgrading an existing installation, **stop the old daemon**: `start` reuses a running one.

```sh
herdr-desktop-pet stop
brew update && brew upgrade herdr-desktop-pet
herdr-desktop-pet --version
herdr-desktop-pet start
herdr-desktop-pet status
```

Verify the CLI reports `0.3.5` and, after restart, `status` shows `app_version: 0.3.5` **and the expected running executable**. The formula puts the app under Homebrew's prefix, **not** `/Applications`, and the CLI on `PATH`; it installs neither Herdr plugin nor patched host. Stop older daemons first; `start` otherwise reuses one. See [0.3 upgrade guidance](docs/releases/0.3.md#upgrade-from-v021-or-beta) for profile isolation and older cask migration. Formula availability depends on actual publication, not this source target.

### Pet without Herdr

For a continuously visible independent pet without a healthy local Herdr server, turn off the new profile's default 30-second disconnect exit **before** starting:

```sh
herdr-desktop-pet settings set exit_with_herdr off
herdr-desktop-pet settings get
herdr-desktop-pet start
herdr-desktop-pet status
```

No Herdr installation, plugin or patch is needed. After starting, `herdr-desktop-pet hide_bubble` optionally hides the empty bubble while retaining the character. If the pet disappears after about 30 seconds, inspect `settings get` and `status`: with `exit_with_herdr` on and no healthy server, it exits after the grace period. Set it off before restarting (using the same actual config/state profile). This switch does **not** override confirmed disable/unlink of all registered local plugin endpoints. [Lifecycle details](docs/usage.md#lifecycle-and-plugin-actions).

### Optional advanced: managed Herdr plugin and attach auto-start

On the intended compatible Herdr host, install the [plugin repository](https://github.com/hanbong5938/herdr-desktop-pet) and explicitly start the pet:

```sh
herdr plugin install hanbong5938/herdr-desktop-pet
herdr plugin action invoke start --plugin desktop-pet
```

Managed installation runs `bash scripts/install.sh`: it tries the manifest-pinned v0.3.5 `.app`-only archive, validating checksums, archive entries, arm64 signature and bundled character. Require a **non-draft v0.3.5 Release with complete archive/checksums and updated installer**; the separately uploaded provenance sidecar does not enter the installer archive. If prebuilt assets are unavailable or invalid, the default installer reports this and builds the checkout from source. Explicit `--prebuilt` fails instead of falling back; `--source` builds the checkout. Plugin installation does not patch or build Herdr. For attach auto-start, prepare the patched host described [here](integrations/herdr/README.md#build-in-separate-checkouts); native manual observation above needs neither.

## Install this checkout from source for managed integration

**For the optional managed attach workflow**, from this repository root first follow the [host build procedure](integrations/herdr/README.md#build-in-separate-checkouts) to patch/build official Herdr 0.9.3 in a separate checkout, run its isolated server, and set `PATCHED_HERDR` to that **built host executable** (for example, `PATCHED_HERDR="$HERDR_SOURCE/target/release/herdr"`). Keep the same intended host/profile/socket for the commands below; an unpatched binary on `PATH` is not a substitute for attach auto-start. Building the pet from source alone does not require a patched host.

```sh
bash scripts/install.sh --source
"$PATCHED_HERDR" plugin link "$PWD" --enabled
"$PATCHED_HERDR" plugin action invoke start --plugin desktop-pet
```

`plugin link` does **not** run the manifest build; build the pet first. Linking/enabling on an already-running host does not itself launch the pet: invoke `start` once, or await a later eligible shell/terminal client attach with `auto_start` enabled. Inspect link warnings: an unknown `client.attached` hook means the host lacks the required patch. Herdr actions require a running enabled host; server startup also runs automatic `ensure`.
Manifest lifecycle and action commands use `scripts/run-native.sh`, selecting `HerdrDesktopPet.app` for a stable manifest version or `HerdrDesktopPetBeta.app` for `-beta.N`; a missing matching bundle fails rather than running the other channel. For direct source automation use `./dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet`; stop an old daemon or isolate **both** config and state paths, then inspect `status` for the actual executable/version. See [source development](docs/development.md#source-development).

## First use

### In-app updates (protocol 3 in this checkout)

**Settings → App updates** offers independent **Automatic checking** (on by default) and **Receive beta updates** (off by default), **Check now**, the running version/origin, policy-specific last successful check, and an explicit action. Stable-only checks select official stable releases; opting in selects the highest eligible stable or beta by semantic version (a final release outranks its same-base beta). Opting out never downgrades an installed beta. Automatic checks run at most daily while the app is open, with separate stable-only and beta-inclusive timestamps. Switching beta policy invalidates the old candidate and checks under the new policy only when automatic checking is on; otherwise use **Check now**. Checks are read-only: neither toggle nor a check installs, restarts, refreshes Brew metadata or changes Git. Moving between release/channel/source and target requires separate native consent for the exact proposal.

The card distinguishes current check or handoff activity from the retained **Previous operation** result; **Show diagnostics** expands selectable paths, hashes, IDs and raw errors. Its short status and helper guidance follow the chosen UI language. A candidate installed on disk is not necessarily applied or running; a source-current or pinned result does not assert the latest public release. Herdr-managed migration pins the official release tag to an immutable commit on the same compatible live host without resetting a checkout. Homebrew keeps the captured manager/prefix and explicitly names the target formula/version; the confirmed operation revalidates metadata and reserves both installation roots without silently uninstalling the original formula. A local checkout rebuilds its on-disk files without Git pull/reset. Only a verified compatible already-installed package may offer **Apply installed update** without rerunning an installer; unknown origins cannot become managed installs implicitly.

Native consent and Prepare recheck pending edits, IME composition and interactions before freezing mutating controls; the original keeps running through the handoff. An ownership ACK is not installation or permission to Stop: only the matching final visible/prepared ACK after accepted Prepare transfers consent, and only a fully delivered accepted reserved Stop commits updater shutdown. Staging runs on a worker so the helper window stays responsive; crossing the native 30-second handoff deadline means **waiting, not cancellation**. The helper checks replacement readiness and records installed and applied separately. Candidate startup uses the exact verified **image and installed origin** pair in the journal/reservation, never an origin inferred from current disk or the original source token. Uncertain or old-protocol reservations remain blocked with evidence intact; recovery does not replay an installer, silently delete a journal, or restart an original automatically. A user Stop prevents revival, but a proven safely stopped transaction can finish reconciliation and release its own reservation. See [update behavior and recovery](docs/usage.md#in-app-updates) and [CLI status and recovery](docs/cli.md#app-update-status-and-recovery) before retrying an uncertain operation.

The published stable `v0.3.3` and paired `0.3.3-beta.1` archives remain immutable matched-protocol-2 app/helper packages: they do **not** gain this checkout's protocol-3 beta setting merely through a check. This checkout's source version or native CLI version alone is not proof of a published protocol-3 release. Adoption requires an explicit source build of this protocol-3 checkout or a separately verified published protocol-3 package. App/helper protocol pairs cannot be mixed; legacy protocol-1/2 records require their matching retained helper, not conversion by the new updater. Check actual [Release assets](https://github.com/hanbong5938/herdr-desktop-pet/releases) for published versions.

## Controls

| Action | Result |
| --- | --- |
| Tap head/body; move across head | React; pet Rubelia. |
| Drag character; use bottom-right grip | Move; resize the character. |
| Drag bubble background or an edge/corner | Move attached character or standalone bubble; resize bubble independently. |
| Left-click menu-bar icon | Open Settings for characters, observation, bubble and lifecycle. |
| Expand bubble, select a local agent card | Open a one-line reply; Enter, Command+Enter or Send submits. Remote cards are read-only. |

IME composition **never submits**; Escape cancels composition first. Search-field Enter/Command+Enter never sends a reply. Character and bubble visibility are independent; hiding both or enabling full-window click-through can make them inaccessible by clicking the desktop. Use the menu-bar icon's **Show and Enable Character** recovery action when needed (the default mode always shows the icon; recovery-only mode shows it when necessary). For full controls, search, saved-machine remote observation and visibility behavior, see the [usage guide](docs/usage.md#controls).

## Optional characters and outfits

The [character gallery](https://hanbong5938.github.io/herdr-characters/) previews optional packs; the [separate repository](https://github.com/hanbong5938/herdr-characters) holds their source and terms. In Settings, choose **Characters → Browse Characters…**; browsing portraits does not download archives. Eligible published official packs have an explicit verified **Download & Apply**. For a local folder or `.herdrchar`, **Import** alone does not activate it: select its installed card/revision and click **Apply**. Rubelia remains the only bundled/default character. See [character packs](docs/usage.md#character-packs) and [authoring](docs/development.md#character-authoring).

## Safety and limitations

- Replies use only the selected **local** Herdr source's `agent.prompt`, not SSH, a shell or raw pane input; remote cards cannot send. Herdr's ACK means submission, **not agent completion**. Handle approval/questions in the actual terminal; check uncertain delivery before any manual resend. Session/pane replacement around submission has a current-occupant race, not an atomic session guarantee. [Delivery details](docs/usage.md#inline-replies-and-message-delivery).
- **Remove Worktree…** is irreversible: confirm the **exact checkout path** and the **entire workspace's** tabs/panes before proceeding. It terminates their processes/agents and removes the linked checkout **including ignored files**; no Trash/undo. It does not delete the main repository or branch. [Removal safeguards](docs/usage.md#worktree-removal).
- Remote observation requires selected saved-machine profiles and a compatible remote Herdr server, **not** a remote pet installation. The local app polls remote state and cannot recover missed brief transitions or send remote prompts. [Observation guide](docs/usage.md#observation-sources).
- Automatic app-update checks are read-only; installing or applying an update requires explicit consent. An updater ownership ACK is **not completion**. For an unknown result, check [in-app update recovery](docs/usage.md#in-app-updates) and [CLI update status and recovery](docs/cli.md#app-update-status-and-recovery) before retrying.

## Configuration and troubleshooting

`HERDR_PLUGIN_CONFIG_DIR` holds `preferences.json`, `lifecycle.json`, managed `characters/` and `menu-bar-icons/`; `HERDR_PLUGIN_STATE_DIR` holds the lock, control socket and `desktop-pet.log`. Without injection, Herdr's XDG plugin locations apply; native `--config-dir` / `--state-dir` can isolate profiles, but injected environment paths take precedence. Back up **both** actual directories before upgrading or switching channels. `lifecycle.json` controls `auto_start` and `exit_with_herdr`, separately from other preferences.

If the pet disappears about 30 seconds after a no-server start, use `settings get` to check `exit_with_herdr` and set it off **before** restarting for standalone use; confirmed disable/unlink of all registered local endpoints still ends this run. If hidden, use the menu-bar recovery above rather than reinstalling. If absent on a managed host, check its `client.attached` warnings, manual `start`/`status`, actual executable/version and daemon log; disabled auto-start can intentionally skip launch. For connection errors inspect `HERDR_SOCKET_PATH` / `HERDR_CLIENT_SOCKET_PATH` and the server actually owning the backend socket; for remote errors inspect the saved profile and local CLI. Run the pet on the **Mac whose desktop should display it**. Provider integrations belong to upstream Herdr; reload existing provider sessions after installing their hooks. See [troubleshooting](docs/usage.md#configuration-and-troubleshooting) and [lifecycle](docs/usage.md#lifecycle-and-plugin-actions).

## Documentation and websites

- [Usage and recovery](docs/usage.md) · [development and installation](docs/development.md) · [native automation CLI contract](docs/cli.md#english) · [patched host integration](integrations/herdr/README.md).
- [Release history](docs/releases/README.md) · [v0.3.5 changes](docs/releases/v0.3.5.md) · [v0.3 upgrade guidance](docs/releases/0.3.md#upgrade-from-v021-or-beta) · [historical beta/preview guidance](docs/migrations/unreleased.md) · [v0.1.11 → v0.2.0 migration](docs/migrations/v0.1.11-to-v0.2.0.md).
- The promotional [English](https://hanbong5938.github.io/herdr-desktop-pet/en/), [Korean](https://hanbong5938.github.io/herdr-desktop-pet/), [Simplified Chinese](https://hanbong5938.github.io/herdr-desktop-pet/zh-Hans/), and [Japanese](https://hanbong5938.github.io/herdr-desktop-pet/ja/) websites describe this **native** app; it does not run in a browser. Website development and deployment: [GitHub Pages guide](docs/development.md#website-github-pages). The separate [character gallery](https://hanbong5938.github.io/herdr-characters/) previews optional packs in English and Korean.

## Artwork and licensing

[Software license](LICENSE.txt) and artwork/model terms are **distinct**: MIT software terms do not relicense character art. Bundled Rubelia's [license](assets/rubelia-default/LICENSE.txt), [attribution](assets/rubelia-default/ATTRIBUTION.txt), [source record](assets/rubelia-default/source-record.json), and [background-repair record](assets/rubelia-default/background-repair.json) remain authoritative. `LICENSE.txt` also retains the original Coding Cat notice; its artwork/source moved to the [character repository](https://github.com/hanbong5938/herdr-characters). Keep each optional pack's own terms with it. Artwork-owner approval does not grant model rights; Qwen's research license is an authoring record, and model engines/weights are not bundled. See [character authoring and licenses](docs/development.md#character-authoring).
