# Herdr Desktop Pet

A native macOS desktop companion for [Herdr](https://herdr.dev). Rubelia lives on your desktop, reacts to touch, displays observed Herdr session activity, and lets you submit text to a selected agent session from its status bubble.

**English** · [한국어](readme.ko.md) · [Releases](https://github.com/hanbong5938/herdr-desktop-pet/releases)

<img src="assets/rubelia-thumbnail.png" alt="Rubelia, the default desktop companion" width="220">

## Features

- Native desktop character with a menu-bar control panel.
- Observe local Herdr sessions by default, with optional saved-machine remote observation.
- Session-aware states: idle, running, waiting, and unknown.
- Readable session cards with task titles, workspace/tab context, source labels, and a separate status label.
- Inline replies beneath selected local agent session cards in the expanded status bubble, included in v0.1.6; remote cards remain read-only. The older v0.1.4 prebuilt retains its message composer.
- Head/body tap reactions, head petting, dragging, and resizing.
- Configurable status bubble placement and visibility.
- Optional status icons and colors, with a status summary that remains visible alongside dialogue.
- Full-window and alpha-mask click-through modes.
- English and Korean UI options.
- Importable PNG and rig character packs with revision history.
- Edit per-character Korean and English dialogue without modifying character packs.

Rubelia is the only bundled character and the default model (`default@0`). Optional characters and wardrobe packs are maintained in the separate [character repository](https://github.com/hanbong5938/herdr-characters) and must be imported. The bubble reports observed Herdr session status; it does not infer provider results. A successful message submission means Herdr acknowledged the prompt, not that the agent finished processing it.

Session cards use the observed terminal title, then a named tab, then workspace/directory context. Missing names receive an explicit fallback; duplicate names gain a visible discriminator. Hover a card for the full title, working directory, and internal IDs. Renaming a title preserves selection and scroll position; disconnected sessions retain their last observed title with an offline status.

Opening the menu-bar control panel requests app activation using the current macOS API, with a compatible activation path on macOS 13. macOS decides whether to grant activation; opening the panel does not guarantee a keyboard-focus transfer from another app.

### Inline replies

The following inline-card behavior is included in the v0.1.6 prebuilt and source builds.

Expand the bubble to see session cards; the reply field is initially hidden, including in the **All** view. Click a local agent session card to open a one-line reply beneath it; clicking another session folds the previous field and opens the new one. Remote cards show read-only feedback instead of a reply field or Send button. Line breaks in any committed input (typing, paste, or text import) become spaces in the one-line reply; active IME preedit remains untouched until committed. Press **Enter** or **Command+Enter**, or click **Send**, to submit. IME composition does not submit; **Escape** cancels active composition first, otherwise it folds only the reply field. Clicking outside the bubble folds the reply unless IME composition is active; clicking elsewhere inside the bubble does not automatically fold it. Neither action collapses the bubble. Drafts are kept in memory per session through folding and bubble collapse, but not across app restarts. Success clears and folds only the matching current reply; if the text was edited or another session selected during sending, the new draft or open field remains. Failure or uncertain delivery preserves the draft and reply state.

During IME composition, card selection, filter changes, and structural row replacement wait without discarding the composing text. When composition ends, the latest filter and still-visible, valid session selection are applied; the completed text stays with its original session and connection generation. Live status and send eligibility continue to update while navigation is deferred.

The reply field keeps a visible border and an opaque, palette-derived background. Its compact native Send button follows the existing send eligibility; live theme changes retain the draft and IME preedit.

### Message delivery (prebuilt and source builds)

Messages go only through the selected local Herdr source's `agent.prompt` API, never through a global CLI, shell, SSH, or raw pane input. Sending is unavailable if Local is excluded from observation, the source is offline, the selected session is stale or not ready, or the server does not support the method (`unsupported_method` is reported rather than falling back). Remote sources are observation-only, including retained remote cards: they cannot send prompts. Approval/question UI must be handled in the actual terminal. This API was verified with Herdr 0.9.2; the 0.9.0 baseline for local observation does not guarantee `agent.prompt` support on the running server. An uncertain delivery result must be checked before manually sending again; there is no automatic retry. The current API addresses the agent occupying a pane without an atomic expected-session guard, so an instantaneous occupant/session replacement cannot be ruled out.

## Requirements

| Component | Requirement |
| --- | --- |
| Platform | Apple Silicon Mac, macOS 13 or later |
| Herdr for this lifecycle feature | **Official Herdr 0.9.3 source with the [supplied client-attach patch](integrations/herdr/client-attached.patch) applied and built** (or a future actual host release advertising this hook) |
| App | Pinned prebuilt v0.1.9 (downloaded by the plugin installer or Homebrew; no build toolchain), or build this checkout with Rust/Cargo, Bun, Node.js/npm, and Xcode Command Line Tools |

**The patch is REQUIRED for client-attach auto-start.** Stock Herdr 0.9.0 and 0.9.3 do not provide `client.attached`; a manifest version floor is not proof of hook support. Apply the supplied patch to official 0.9.3 source and build/run that host following its build instructions; inspect plugin link warnings and verify the `client.attached` subscription is accepted. Do not treat an unknown-hook warning as success. Intel Macs, Windows, and Linux are not supported by the native distribution.

## Install as a Herdr plugin (recommended)

Install from the public [plugin repository](https://github.com/hanbong5938/herdr-desktop-pet) like any other Herdr plugin:

```sh
herdr plugin install hanbong5938/herdr-desktop-pet
herdr plugin action invoke start --plugin desktop-pet
```

Herdr clones the repository into its plugin directory and runs the manifest's `[[build]]` step, `bash scripts/install.sh`. The installer downloads the release pinned to the manifest `version` (v0.1.9) anonymously over HTTPS with `curl`, verifies its SHA-256, archive paths and entry types, arm64 architecture, code signature, and default character, and installs it into the plugin directory's `dist/`. No GitHub login or build toolchain is needed in the normal case, and nothing is installed outside the plugin directory. If the pinned prebuilt is unavailable or fails verification, the installer prints a notice and falls back to a source build, which requires the source toolchain listed above. The app is **ad-hoc signed, not notarized**; because it is fetched by `curl` rather than a browser or cask download, it carries no quarantine attribute and macOS shows no Gatekeeper prompt. Client-attach auto-start still requires the supplied host patch: run these commands with the patched Herdr host described above.

The repository is tagged `herdr-plugin` for automatic discovery in the [Herdr marketplace](https://herdr.dev/plugins/). The index refreshes every 30 minutes; this is an unreviewed community listing.

## Install with Homebrew

Install the prebuilt app and `herdr-desktop-pet` CLI from the [personal tap](https://github.com/hanbong5938/homebrew-tap) formula; no GitHub login or source-build toolchain is required:

```sh
brew install hanbong5938/tap/herdr-desktop-pet
herdr-desktop-pet start
herdr-desktop-pet status
```

The Homebrew formula installs `HerdrDesktopPet.app` inside Homebrew's prefix, not the system Applications folder, and puts the `herdr-desktop-pet` CLI on `PATH`. This installs the app only: it does not register the Herdr plugin or install a patched Herdr host. Use the plugin installation above (or the source installation below) for plugin startup hooks; client-attach auto-start still requires the supplied host patch. The app is **ad-hoc signed, not notarized**; because the formula is not a cask download, Homebrew does not quarantine it and macOS shows no Gatekeeper prompt.

```sh
brew upgrade herdr-desktop-pet
brew uninstall herdr-desktop-pet
```

Uninstalling keeps your character packs, preferences, and lifecycle state.

Earlier versions shipped a Homebrew cask that copied the app to `/Applications`. To migrate a previous cask install, remove it and install the formula (your data is kept):

```sh
brew uninstall --cask herdr-desktop-pet
brew install hanbong5938/tap/herdr-desktop-pet
```

## Install this feature from source

For development checkouts, `plugin link` does not run the manifest's `[[build]]`, so build the checkout yourself. In this feature's source checkout, with the patched Herdr host running in an isolated profile (follow the [patched-host deployment guide](integrations/herdr/README.md) and use that binary explicitly, rather than an unpatched `herdr` on `PATH`):

```sh
bash scripts/install.sh --source
"$PATCHED_HERDR" plugin link "$PWD" --enabled
"$PATCHED_HERDR" plugin action invoke start --plugin desktop-pet
```

The installer installs pinned JavaScript dependencies, builds the Rust executable, packages the native rig runtime and creator resources, and validates the app. Linking/enabling on an already-running server does not itself launch the pet: invoke `start` once, or wait for a subsequent successful shell/terminal client attach when `auto_start` is on. Server startup also runs automatic `ensure`. Inspect link warnings: an unknown `client.attached` hook means the host lacks the required patch. Herdr plugin actions require a running, enabled host; direct native settings commands below work without one.

The **v0.1.9 prebuilt app includes the five-fold-face ten-pose Rubelia default, lifecycle settings, editable dialogue, inline replies, and status indicators**. v0.1.9 also adds the source-checkout [local Qwen character-production skill](.agents/skills/create-pet-character/SKILL.md), forked from DAEMONLET with its MIT license and upstream record preserved. v0.1.7 fixed the packaged CLI so it finds its bundled default character when launched through a symlink on `PATH`. v0.1.8 keeps the character menu usable after pack operations finish or age out of the status cache, keeps a failed dialogue load visible while you type (reopen the editor to retry, for example after another process releases the character store), and keeps session card titles stable during IME composition. `bash scripts/install.sh --prebuilt` downloads the version pinned by this checkout's `herdr-plugin.toml` anonymously over HTTPS with `curl` (no GitHub login) from the checkout's git `origin` repository (falling back to `HERDR_PET_REPOSITORY`, then `hanbong5938/herdr-desktop-pet`) and validates SHA-256, archive paths and entry types, arm64 architecture, code signature, and default character; it never falls back to a source build. Running the prebuilt app requires no source-build toolchain; client-attach auto-start still requires the patched Herdr host described above. Without an option, `scripts/install.sh` installs the pinned prebuilt and falls back to a source build only if the prebuilt is unavailable or invalid; use explicit `--source` to build local changes. App bundles are **ad-hoc signed, not Developer ID signed or notarized**; installer downloads are not quarantined, so macOS shows no Gatekeeper prompt for them.

The older **v0.1.4 prebuilt message composer** differs from v0.1.6 inline replies: **Enter** inserts a newline, **Command+Enter** sends, and **Escape** collapses the bubble (during IME composition, Escape cancels composition first). Upgrade to v0.1.6 or use `--source` for inline-card replies.

## Controls

| Interaction | Effect |
| --- | --- |
| Menu-bar icon | Character selection and dialogue editing, observation sources, bubble and lifecycle settings, and UI language |
| Tap the head or body | Trigger a reaction |
| Move back and forth on the head | Pet the character |
| Drag the body or background | Move the character |
| Option-drag anywhere | Move the character |
| Bottom-right grip | Resize the character |
| Expanded bubble session card (`--source` only) | Open a one-line reply beneath the selected local agent session; switching cards waits for active IME composition to end, while remote cards show read-only feedback |
| Inline reply (`--source` only) | Enter, Command+Enter, or Send submits to the selected local session; IME composition does not submit |
| Escape in the reply / click outside the bubble (`--source` only) | Escape cancels active IME composition first; otherwise either folds only the reply field. Inside-bubble clicks do not automatically fold it; outside clicks do not fold during composition |
| Full-window click-through | Pass clicks through both the pet and bubble windows, disabling their interaction |
| Alpha click-through | Pass clicks through transparent artwork regions |

The menu panel uses native dropdowns for bubble theme and UI language. Choose **System** to follow the system language, or select Korean or English explicitly. Theme and language selections are saved when changed; custom bubble colors keep their **Apply colors** action.

In the **Characters** tab, clicking a character row or revision stages a candidate; it does not change the running character. Review the candidate and click **Apply** to select it. **Cancel** clears an unsubmitted selection; closing and reopening the panel preserves it. While any pack mutation is pending, selection, Apply, Cancel, Import, Update, Remove, and Restore are blocked; read-only inspection and diagnostics remain available. File-picker and confirmation returns are checked again for busy work and listing changes rather than silently using a newer generation. If an operation is canceled or fails, its candidate remains available. For an unknown submitted result, read the full operation ID in the footer tooltip or diagnostics and run `pack status OPID` instead of retrying blindly. A rejected submission is distinguished from an unknown submitted operation. The latest UI result remains in diagnostics after Apply completes or a new candidate is staged. Successful inactive imports, updates, and removals are complete even when they do not activate a renderer. The panel distinguishes the live character from the committed selection separately from operation completion.

**Bubble → Status icons and colors** is enabled by default. Changes apply immediately and survive restarts; turning it off restores the previous text-only presentation. Running is blue, waiting for attention is amber, validated success has a green check, and failure is red. Plain `Done` and idle states without an accepted result remain neutral rather than implying success. An unchanged validated terminal report for the same session retains its result across idle events and snapshots. New activity, ambiguous session identity, report loss, or disconnection clears the previous result indicator; resolving ambiguity does not re-accept the same cached terminal report. A new source remains offline until its first coherent snapshot, including an empty snapshot. The summary shows a green check only when every observed session has a validated successful result, independently of the card filter or display limit.

With indicators enabled, every state also has a text label. Decorative icons are excluded from the accessibility tree, and each session card exposes its full title and status without duplicate child elements.

While a native dropdown, scroller, or drag is tracking, bubble display and placement changes are deferred until tracking ends, then apply the latest settings and status. Switching indicators on or off preserves card selection, the filter, and the message draft.

Alpha click-through uses pointer polling. Leave it off if a click-routing race is unacceptable.

### Custom dialogue

Click **Edit dialogue…** in the **Characters** tab to edit in a separate, resizable window. Choose any available character and revision without activating it in the renderer; select Korean or English and one of eight sidebar events: idle, running, waiting, unknown, head tap, body tap, petting, or observed task completion. Event badges mark unsaved edits. With no override, the editable field starts with the original text; expand the separate original reference to compare. Editing language does not change the UI language.

The editor uses the settings panel's dark theme, with character/language controls at the top, events on the left, editing and original-reference areas in the center, and a fixed status/action footer. The default content area is 760×560; the minimum window size is 620×480. Hover long names in the character selection footer or long editor errors to read their full text.

- Enter multiple lines and click **Save dialogue** or press **Command+S** to save one language/event entry. Each entry allows up to 2048 UTF-8 bytes, not 2048 characters. Blank or whitespace-only text removes that override. A successful save for the active character ID refreshes applicable bubble text even when editing a historical revision; editing an inactive character does not activate it.
- **Reset this entry** confirms loss of an unsaved draft before removing that override. **Reset all character dialogue…** always requires confirmation and removes that character's overrides in both languages. Missing entries retain the existing pack/default/status fallback; an empty entry does not suppress the bubble.
- Drafts are isolated by character, language, and event. Switching context or UI language and closing/reopening the window preserves drafts in memory during the app session; only saved entries survive restart. **Command+Z** undoes and **Shift+Command+Z** redoes within the current editing context. A failed save keeps the draft and previously committed dialogue.
- Saved entries live in `preferences.json`, not pack files. Built-in and imported characters are keyed by character ID: revisions share overrides, including after updates and restores. External `--assets` characters use a separate canonical-path namespace and do not share entries with the built-in character. The active external character remains editable under its canonical identity captured at activation even if its source folder is renamed while the app runs; on restart with a changed `--assets` path, stored dialogue remains under the original path key and is not automatically migrated.
- If corruption of both managed registry files puts the store into its read-only fallback, the built-in original dialogue remains readable at the current listing generation without activating it or repairing the registry. Unavailable managed revisions remain unavailable.

Custom text replaces only the corresponding dialogue entries. It does not change observed session counts, state transitions, reaction timing, or disconnected-host notices.

## Observation sources

The **Observation sources** section in the menu-bar settings includes **Local** by default. **Remote** is off by default. To observe a remote session:

1. In an interactive terminal, add and authenticate an SSH machine with Herdr (for example, `herdr machine add workbox`). Follow [Herdr's saved-machine setup guide](https://herdr.dev/docs/0.9.2/connecting-machines/) for SSH access, remote-session selection, and any prompted server setup.
2. Open the pet's **Observation sources** settings, turn on **Remote**, and select the desired enabled machine profiles. Each profile observes its one saved remote Herdr session, not every session on that host. The remote host needs a compatible running Herdr server, but does **not** need the desktop-pet app or plugin installed.

The pet refreshes Herdr's saved-machine catalog every 5 seconds, even when Remote is off, and polls only selected, enabled profiles every 5 seconds when Remote is on. It remembers profile selections across app restarts and when Remote is switched off; renaming a profile keeps its selection. Disabling or removing a saved profile excludes it from observation without stopping remote sessions or processes. Local and each remote profile have separate source labels on session cards, so matching session IDs on different servers remain distinct.

The Local/Remote switches and profile selections determine which sources contribute to cards, counts, overall phase, and completion/outcome reactions. The existing card status filter only narrows which included cards are displayed; it does not change which sources are observed. Turning off every source shows **Unknown**, not a disconnected warning for a source you chose to exclude.

Remote snapshots are polled rather than streamed: transitions shorter than the 5-second interval can be missed, including brief completions. If a selected machine becomes unavailable, its last observed cards are marked **Offline**, and retained sessions count as unknown rather than live; check the error in **Observation sources**. Once it reconnects, or when a source is reselected, the first fresh snapshot is a baseline and does not replay old completion reactions. To recover SSH authentication, run `herdr machine reconnect '<profile-id>'` yourself in a terminal, using the ID from `herdr machine list`. The pet does not prompt for credentials, perform remote setup, or silently fall back to Local for a failed remote request.

Local observation still supports Herdr 0.9.0 or later. Remote observation additionally needs a local Herdr CLI supporting [saved-machine API forwarding](https://herdr.dev/docs/0.9.2/cli-reference/#saved-ssh-machines) (`herdr --machine <profile-id> api snapshot`) and a compatible running remote Herdr server; Herdr 0.9.2 documents that CLI capability. A missing or incompatible CLI is reported in settings rather than changing the local observation minimum requirement. **Lifecycle client-attach auto-start has the separate patched-host requirement above.**

## Lifecycle settings

The Settings tab has two independent switches: **Auto-start when Herdr starts** (`auto_start`) and **Quit when all Herdr servers disconnect** (`exit_with_herdr`). Both default to on for a new profile. Existing `lifecycle.json` profiles migrate legacy boolean `enabled` to `auto_start` (including `false`), and default missing `exit_with_herdr` to off to preserve their prior independent-running behavior. Both settings live only in `lifecycle.json`, not `preferences.json`; existing endpoints, startup metadata, and unrelated fields are preserved. Invalid values/JSON produce an error instead of being replaced with defaults; a failed save leaves the previous switches and running policy unchanged and shows an error.

`auto_start` on starts an absent pet at server startup or a successful shell/terminal client attach. Automatic `ensure` with auto-start off successfully skips an absent pet; it may register an endpoint for an already-running pet but never forces a hidden pet visible. Manual `start` works with auto-start off and does not change either setting. `stop` and the native **Quit** end only this run (including a pending startup); `restart` starts manually again and preserves settings. If auto-start remains on, the next qualifying server startup/client attach can start the pet again. Turn off `auto_start` to disable future automatic starts. Herdr plugin disable/unlink is a separate host action, not a pet setting; disabled plugins do not receive actions/hooks, and confirmed disable on all registered endpoints terminates an already-running pet immediately.

When `exit_with_herdr` is on, the pet quits 30 seconds after the last **healthy server connection** disappears unless a healthy connection returns. An initial launch with no reachable server gets the same finite 30-second grace; one remaining healthy server keeps it alive. Client detach alone is not server loss; stale saved endpoints and failed retries do not extend grace. Switching exit off cancels a live countdown; switching it on while disconnected starts a fresh 30 seconds. With exit off, a manually started pet can continue without Herdr. Changing auto-start does not itself stop or show the current pet. `show`, `hide`, and `toggle` change visibility only; `restart` preserves position, scale, and preferences.

## Plugin actions

```sh
herdr plugin action invoke status --plugin desktop-pet
herdr plugin action invoke settings --plugin desktop-pet
herdr plugin action invoke restart --plugin desktop-pet
herdr plugin action invoke stop --plugin desktop-pet
```

The global `settings` action opens only the lifecycle settings window even with the pet stopped. For offline access, use the executable installed from this source checkout directly (no running pet or enabled/running Herdr host needed):

```sh
PET="./dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet"
"$PET" settings
"$PET" settings get
"$PET" settings set auto_start off
"$PET" settings set exit_with_herdr on
```

`settings get`/`set` return structured JSON; `set` accepts only `auto_start` or `exit_with_herdr` and exactly `on` or `off`, changes no other setting and does not launch the pet. Invalid keys/values, malformed arguments, and save failures return errors rather than silently resetting settings. `status` reports `auto_start` and `exit_with_herdr`. Additional action IDs: lifecycle `start`, `ensure`, `stop`, `restart`, `status`, `settings`; visibility `show`, `hide`, `toggle`; click-through `passthrough`, `alpha_passthrough`; bubble `show_bubble`, `hide_bubble`, `bubble_above`, `bubble_below`, `bubble_left`, `bubble_right`, `bubble_auto`; geometry `reset`, `bigger`, `smaller`.

## Character packs

Use the **Characters** menu, or run the native CLI from the checkout:

```sh
PET="./dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet"
"$PET" pack list
"$PET" pack validate --path /absolute/path/to/character
"$PET" pack import --path /absolute/path/to/character.herdrchar
"$PET" pack select CHARACTER_ID
"$PET" pack update CHARACTER_ID --path /absolute/path/to/updated-character
"$PET" pack restore CHARACTER_ID --revision 1
"$PET" pack remove CHARACTER_ID
"$PET" pack select default
```

Replace the example paths, `CHARACTER_ID`, and revision number with your own values.

- Import accepts a character directory or `.herdrchar` archive; importing does **not** select it.
- In the **Characters** tab, row/revision choices only stage a candidate until **Apply**. Native CLI `pack select` immediately selects the newest revision, and `pack restore` immediately selects an existing historical revision; these commands do not require UI Apply.
- Removing the active pack switches back to Rubelia.
- The built-in `default@0` cannot be imported, updated, or removed.
- Keep operation IDs. If a mutation's outcome is uncertain, run `"$PET" pack status OPERATION_ID` instead of blindly repeating it.

For authoring, see the [character creator guide](.agents/skills/character-creator/SKILL.md) and `tools/character-pack.py`. The packaged app includes them at `Contents/Resources/creator/SKILL.md` and `Contents/Resources/creator/character-pack.py`. From a source checkout, the [local Qwen + See-through production skill](.agents/skills/create-pet-character/SKILL.md) generates ten independent poses from an approved illustration; its external engines and model weights are not bundled. Rubelia is the bundled rig template. PNG templates, Aurora, Coding Cat, optional Rubelia outfits, and their artwork generators are maintained in [herdr-characters](https://github.com/hanbong5938/herdr-characters); use `--templates-root PATH` to supply an external template.

## Configuration and troubleshooting

- `HERDR_PLUGIN_CONFIG_DIR`: `lifecycle.json` (both lifecycle switches), `preferences.json` (other preferences), and managed characters.
- `HERDR_PLUGIN_STATE_DIR`: process lock, control socket, and daemon log (`desktop-pet.log`).
- Without injected paths, the app uses Herdr's XDG config/state plugin directories.
- Native commands accept `--config-dir` and `--state-dir` for isolated profiles; injected environment paths take precedence.
- If the pet does not appear after linking, check for an unknown-hook warning, invoke `start`, then inspect `status` and the daemon log. An off `auto_start` is an intentional automatic skip.
- For remote observation errors, check **Observation sources** and the saved profile's status; see [Observation sources](#observation-sources) for authentication recovery and CLI compatibility.
- If a release download fails, check network access to github.com. `--prebuilt` does not fall back; the default mode falls back to a source build, which requires the source toolchain.
- Use Herdr's upstream provider integrations. Restart/reload existing provider sessions after installing their upstream lifecycle hooks.

## Development

After a source installation:

```sh
bun run check
bun run test:native
```

Repository layout:

| Path | Purpose |
| --- | --- |
| `herdr-plugin.toml` | Herdr plugin lifecycle and action definitions |
| `native/` | Rust app, native UI, session handling, and character management |
| `native/rig/`, `web/rig/` | Rig rendering, decoding, and authoring support |
| `assets/` | Bundled Rubelia default and menu thumbnail only |
| `tools/` | Generic character authoring and rig inspection utilities |
| `scripts/` | Installation, native builds, and app packaging |
| `.github/workflows/release.yml` | Release packaging workflow |

## Artwork and licensing

The bundled Rubelia's [license](assets/rubelia-default/LICENSE.txt), [attribution](assets/rubelia-default/ATTRIBUTION.txt), and [source records](assets/rubelia-default/source-record.json) travel with the pack. [LICENSE.txt](LICENSE.txt) retains the original Coding Cat notice; Coding Cat and its drawing source now live in the separate character repository. These notices do not establish a blanket license for every repository component. Keep licenses, attribution, and source-term files with imported packs.

In v0.1.9, the approved 2026-10-02 illustration retains its outfit, body and framing while a local Qwen-Image-2.1 face-only edit uses Rubelia's five-fold portrait as the identity reference. The selected seed is `2026100502`; the source record reports unchanged authority RGBA outside the edit-mask support and unchanged full alpha. That corrected illustration supplies `waiting` and the drawing authority for nine newly generated whole poses, each with its own source-derived layered PSD, expressions, rig and complete motion. The existing `default@0` and Rubelia manifest identity are unchanged, and the thumbnail comes from the native waiting-pose capture. Known limitations: writing, failed and bored have slightly more downcast eyes than waiting, and small opaque white fragments remain between some hair strands. The source records preserve the original private-generation request separately from this release authorization. Existing imported character selections are not replaced. Artwork ownership approval does not grant model rights; the retained Qwen research license is an authoring record, and no model weights or engines ship with the app.

