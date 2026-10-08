# Herdr Desktop Pet

A native macOS desktop companion for [Herdr](https://herdr.dev). Rubelia lives on your desktop, reacts to touch, displays observed Herdr session activity, and lets you submit text to a selected agent session from its status bubble.

**English** · [한국어](readme.ko.md) · [Version documentation](docs/releases/README.md) · [Releases](https://github.com/hanbong5938/herdr-desktop-pet/releases)

> **v0.3.2 is the stable source and default/prebuilt target.** This checkout selects `0.3.2` and integrates session Search/Sort/Running first, native CLI automation, conflict-aware dialogue drafts, eight-way bubble resizing, the official character browser/verified Download & Apply, installation-aware protocol-2 updates and worker-only JIT decoding improvement. [v0.3.2 notes](docs/releases/v0.3.2.md) · [bilingual upgrade guidance](docs/releases/0.3.md). The v0.2.1 Arin compatibility fix (app v0.2.1+ required) remains included; prior releases and beta assets are unchanged. A public v0.3.2 install requires the exact non-draft Release and complete uploaded archive/checksums plus an updated installer/formula; confirm them in the [Release catalog](https://github.com/hanbong5938/herdr-desktop-pet/releases) rather than inferring publication from these manifests. `--source` builds only this checkout. Stop an older daemon or isolate config **and** state; inspect `status` for the running executable. Historical isolated source checks are not integrated/public release verification.


<img src="assets/rubelia-thumbnail.png" alt="Rubelia, the default desktop companion" width="220">

Only Rubelia is bundled. [Browse optional character previews](https://github.com/hanbong5938/herdr-characters#character-previews).

## Features

- Native desktop character with a menu-bar control panel.
- Observe local Herdr sessions by default, with optional saved-machine remote observation.
- Session-aware states: idle, running, waiting, and unknown.
- Readable session cards with task titles, workspace/tab context, source labels, and a separate status label.
- Inline replies beneath selected local agent session cards in the expanded status bubble, shipped since v0.1.6; remote cards remain read-only. See the [v0.1.4 → v0.1.6 migration](docs/migrations/v0.1.4-to-v0.1.6.md).
- Head/body tap reactions, head petting, dragging, and resizing.
- Configurable status bubble placement and visibility, plus independent edge/corner drag resizing in v0.3.2.
- Optional status icons and colors, with a status summary that remains visible alongside dialogue.
- Full-window and alpha-mask click-through modes.
- English and Korean UI options.
- Importable PNG and rig character packs with revision history, plus a native installed/official character browser and explicit verified official Download & Apply in v0.3.2.
- Independent character and bubble visibility, with a standalone bubble and selectable always-visible/recovery-only menu-bar icon and custom PNG artwork.
- Compact local/remote observation settings, copy-only reconnect guidance, and standalone remote-only watcher support.
- Contextual Settings and guarded linked-worktree removal for eligible local worktrees; removal deletes ignored files too.
- Edit per-character Korean and English dialogue without modifying character packs.
- v0.3.2 app update card: automatic read-only checks, installation-aware explicit rebuild/upgrade/apply and durable standalone recovery.

Rubelia is the only bundled character and the default model (`default@0`). Optional characters and wardrobe packs are maintained in the separate [character repository](https://github.com/hanbong5938/herdr-characters). The earlier public v0.2.1 app requires manual import; the v0.3.2 browser offers an in-app official catalog and explicit verified download/apply for supported published packs. The bubble reports observed Herdr session status; it does not infer provider results. A successful message submission means Herdr acknowledged the prompt, not that the agent finished processing it.

Session cards use the observed terminal title, then a named tab, then workspace/directory context. Missing names receive an explicit fallback; duplicate names gain a visible discriminator. Hover a card for the full title, working directory, and internal IDs. Renaming a title preserves selection and scroll position; disconnected sessions retain their last observed title with an offline status.

Opening the menu-bar control panel requests app activation using the current macOS API, with a compatible activation path on macOS 13. macOS decides whether to grant activation; opening the panel does not guarantee a keyboard-focus transfer from another app.

### Session list search and ordering (v0.3.2)

These controls belong to v0.3.2 source, **not** the published v0.2.0/v0.2.1 or immutable old beta binaries. Use `--source` for this checkout or confirm public v0.3.2 assets and the stable installer before relying on a prebuilt; an earlier beta or v0.2.1 binary does not gain the new controls. Programmatic source evidence is not physical GUI/IME certification.

Use **Search** to narrow cards by a case-insensitive substring of their title, tab or workspace name, agent name, source display name, full working directory, or fallback name (surrounding search whitespace is trimmed). IDs and socket paths are not searchable. The **Status** filter and search intersect; neither changes the observed overall status summary or reactions. The **Sort** menu contains **Stable** (original order), **Name**, and **Source**, followed by independent **Running first** below a separator. Running cards lead across all observed sources before the selected sort orders each group. The closed menu title and accessibility value show both choices. Existing saved combinations remain valid; this is not full sorting by every status. The GUI displays at most 128 cards after filtering and sorting; matched/omitted counts include matches beyond that display cap. Unlike the GUI cap, native CLI `sessions list --limit` sets page size and collects coherent pages.

Search starts empty, Status at **All**, Sort at **Stable**, and Running first off. Only a successfully saved Sort/Running first choice survives restart; if saving fails, the controls revert. Search and Status are memory-only. Filtering a selected reply out folds its field but preserves its session draft in memory; clearing search does not reopen it automatically. While Search has focus, **Enter** and **Command+Enter** never send a reply. **Escape** cancels active search IME composition first, then clears a nonempty query, then leaves the empty Search field without collapsing the bubble. Search and reply composition separately defer structural list changes until composition ends; live statuses continue updating.

### Inline replies

Inline-card replies shipped in v0.1.6 and remain in stable v0.2.0, whose native layout fixes clipped reply glyphs and whose responder handles Command-A/C/X/V clipboard shortcuts. Actual physical keyboard and IME behavior depends on the local GUI session; programmatic AppKit checks are not physical-input certification.

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
| App | v0.3.2 is the stable source/default-prebuilt target (verify complete public Release assets and updated installer/formula); source builds require Rust/Cargo, Bun, Node.js/npm and Xcode Command Line Tools. v0.2.1 is the prior public stable patch |

**The patch is REQUIRED for client-attach auto-start.** Stock Herdr 0.9.0 and 0.9.3 do not provide `client.attached`; a manifest version floor is not proof of hook support. Apply the supplied patch to official 0.9.3 source and build/run that host following its build instructions; inspect plugin link warnings and verify the `client.attached` subscription is accepted. Do not treat an unknown-hook warning as success. Intel Macs, Windows, and Linux are not supported by the native distribution.

## Install as a Herdr plugin (recommended)

Install from the public [plugin repository](https://github.com/hanbong5938/herdr-desktop-pet) like any other Herdr plugin:

```sh
herdr plugin install hanbong5938/herdr-desktop-pet
herdr plugin action invoke start --plugin desktop-pet
```

Herdr clones the repository into its plugin directory and runs the manifest's `[[build]]` step, `bash scripts/install.sh`. The manifest pins `0.3.2`; the successful prebuilt path requires the actual published v0.3.2 assets. It fetches anonymously over HTTPS with `curl`, verifies SHA-256, archive paths and entry types, arm64 architecture, code signature and default character, then installs under the plugin directory's `dist/`. If unavailable or invalid, optionless installation prints a notice and falls back to building this checkout from source with the toolchain above; explicit `--prebuilt` does **not** fall back. A fallback build is not proof of public prebuilt availability. The app is ad-hoc signed, not notarized; a `curl` plugin installation does not attach browser quarantine. Client-attach auto-start still needs the patched Herdr host.

The repository is tagged `herdr-plugin` for automatic discovery in the [Herdr marketplace](https://herdr.dev/plugins/). The index refreshes every 30 minutes; this is an unreviewed community listing.

## Install with Homebrew

Install the v0.3.2 stable app and CLI through the [personal tap](https://github.com/hanbong5938/homebrew-tap) **when its formula and the complete public v0.3.2 archive/checksums are available**. Check the [actual Release catalog](https://github.com/hanbong5938/herdr-desktop-pet/releases) and formula version rather than this source manifest; v0.2.1 remains the prior published stable patch:

```sh
brew install hanbong5938/tap/herdr-desktop-pet
herdr-desktop-pet --version
herdr-desktop-pet start
herdr-desktop-pet status
```

For an existing stable installation, **stop the running daemon first**; `start` reuses a running daemon, so `--version` alone does not prove the app was replaced:

```sh
herdr-desktop-pet stop
brew update && brew upgrade herdr-desktop-pet
herdr-desktop-pet --version
herdr-desktop-pet start
herdr-desktop-pet status
```

Confirm the installed CLI `--version` reports `0.3.2`, and after stopping/restarting the daemon `status` reports `app_version: 0.3.2` and the expected running executable. Do not assume the formula changed from source manifests alone. If Homebrew refuses to load the formula, scope trust to this formula: `brew trust --formula hanbong5938/tap/herdr-desktop-pet`. The formula installs `HerdrDesktopPet.app` under Homebrew's prefix, not `/Applications`, and puts the CLI on `PATH`. It installs only the app, not the Herdr plugin or patched host; plugin installation is needed for lifecycle hooks and the host patch for client-attach auto-start. The app is ad-hoc signed, not notarized; formula installation does not quarantine it. `brew uninstall herdr-desktop-pet` keeps character packs, preferences and lifecycle state. See the [0.3 upgrade guidance](docs/releases/0.3.md#upgrade-from-v021-or-beta).

Earlier versions shipped a Homebrew cask that copied the app to `/Applications`. To migrate a previous cask install, remove it and install the formula (your data is kept):

```sh
brew uninstall --cask herdr-desktop-pet
brew install hanbong5938/tap/herdr-desktop-pet
```

## Historical beta channel (not the stable install)

For Apple Silicon on macOS 13+, immutable [`beta/0.2.0-beta.3`](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/beta%2F0.2.0-beta.3) and the separate rolling beta Homebrew formula remain a **historical test channel**, not the stable v0.2.0 formula or default installer. Beta1 contained reply clipping; beta2 fixed clipping and malformed-link scanning but lacked reply Command-A/C/X/V dispatch; beta3 added those shortcuts. Stable v0.2.0 includes the beta3 native fixes. For beta3 → stable, stop beta, back up the shared profile, and install/upgrade **the stable formula**, not the beta formula; follow the [migration guide](docs/migrations/v0.1.11-to-v0.2.0.md). Programmatic AppKit checks do not certify physical keyboard/IME, picker, alert-key, pointer, or VoiceOver behavior.

The historical beta formula installs `HerdrDesktopPetBeta.app` inside Homebrew's prefix, not `/Applications`; `herdr-desktop-pet-beta` is separate from stable `herdr-desktop-pet`. If you are currently running beta3 on the shared profile, stop it before installing or starting stable:

```sh
herdr-desktop-pet-beta stop
brew install hanbong5938/tap/herdr-desktop-pet
herdr-desktop-pet --version
herdr-desktop-pet start
herdr-desktop-pet status
```

If the stable formula is already installed, stop that daemon too and use `brew update && brew upgrade herdr-desktop-pet` instead of `brew install`. Back up the real config **and** state directories, including `preferences.json`, `lifecycle.json`, `characters/`, and managed `menu-bar-icons/`; isolated beta profiles must use matching paths for `stop` and new `start`. Do not run beta and stable concurrently unless both config and state are isolated. The immutable beta3 archive SHA-256 `14bf452aa492fa520a4979ca3e18711e505c52c178c1e1e2edb355b2b07b0920` and publication/install evidence belong to the [historical beta guide](docs/migrations/unreleased.md#optional-manual-beta-test), not the stable asset checksum or proof. A manually downloaded beta bundle is ad-hoc signed and may require Gatekeeper approval; remove only beta with `brew uninstall herdr-desktop-pet-beta` if no longer needed. Test worktree removal only on a disposable linked checkout.

## Install this checkout from source

For development checkouts, `plugin link` does not run the manifest's `[[build]]`, so build the checkout yourself. v0.3.2 includes the earlier v0.2.0 native groups and v0.2.1 Arin fix plus the new capabilities; `--source` builds the actual checkout, not a different release. For client-attach auto-start, use the patched Herdr host in an isolated profile (follow the [deployment guide](integrations/herdr/README.md) and specify that binary rather than unpatched `herdr` on `PATH`):

```sh
bash scripts/install.sh --source
"$PATCHED_HERDR" plugin link "$PWD" --enabled
"$PATCHED_HERDR" plugin action invoke start --plugin desktop-pet
```

The installer installs pinned JavaScript dependencies, builds the Rust executable, packages the native rig runtime and creator resources, and validates the app. Linking/enabling on an already-running server does not itself launch the pet: invoke `start` once, or wait for a subsequent successful shell/terminal client attach when `auto_start` is on. Server startup also runs automatic `ensure`. Inspect link warnings: an unknown `client.attached` hook means the host lacks the required patch. Herdr plugin actions require a running, enabled host; direct native settings commands below work without one.

Use the executable `./dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet` for source-checkout automation; a separately installed v0.3.2 app/CLI can also provide these commands when its public archive/formula have been confirmed. Stop the existing pet first, or use consistent isolated config **and** state paths for `start`, commands and `stop`. A source CLI's `0.3.2` version alone does not establish a public asset or replace an older running daemon; inspect `status` for the executable.

### In-app updates (v0.3.2)

**Settings → App updates** shows automatic checking (on by default), **Check now**, the running version and installation origin, last successful check, and an explicit origin-specific action. Automatic checks run at most daily while the app is open. Automatic and manual checks are read-only: neither installs, restarts, updates Brew metadata, nor changes Git. There is no beta switch or silent channel/ref change. An action needs separate native consent.

- Herdr-managed updates use the *original injected* host plugin config and global Herdr registry, not a named socket's parent or an override pet profile. The recorded enabled source/ref, checkout root and plugin subdirectory stay distinct; a plugin in a subdirectory does not make that subdirectory the whole installation. The same live compatible Herdr host (at least 0.9.3 for this updater), not just a newer CLI on disk, must corroborate the source before an install is offered.
- Homebrew retains the captured manager prefix and exact stable or beta formula/channel. Its physical replacement boundary is that formula's Cellar subtree across kegs, not the whole prefix: a sibling beta formula does not block stable, while another live profile on the same formula does. Checks use cached metadata; only a separately confirmed upgrade may refresh it and upgrade that same formula.
- A local checkout explicitly rebuilds files already on disk, without Git pull/reset or changing refs. An identical rebuilt executable is acceptable only if the replacement instance is actually ready.
- A verified externally installed, signed, protocol-2-compatible package can offer **Apply installed update** without running an installer. Manual or unknown origins do not silently become Herdr or Homebrew installations.

Native consent and Prepare recheck unsaved drafts, IME composition, pending work and active gestures, then freeze mutating controls while the exact original instance remains running. An ownership ACK means the helper owns the handoff; it does **not** mean installation, Stop, or Quit. Only a matching reserved Stop whose accepted reply was fully delivered may commit updater shutdown. A failed reply thaws the original without starting an installer; an ordinary user Stop/Quit wins over updater restart.

Preparation also guards dedicated dialogue undo/redo and browser search/filter/local-folder actions, including AX and internal callbacks, and restores the previous native control states and editor history after safe cancellation/reconciliation. A proven failure before the helper is spawned is retryable after repairing the cause and performing a fresh Check; spawned or uncertain ownership remains blocked until recovery proves it safe.

The standalone signed helper must be outside the mutable physical installation, as must the selected configuration, state and custom assets. Another live profile sharing that physical installation blocks replacement. The helper preserves the selected profile/backend/assets, checks the new instance's actual readiness, and reports installed and applied separately; **Applied** does not imply “latest public release.” Unknown, active, unreadable or old-protocol reservations retain their evidence and block an unsafe new operation, never trigger automatic manager replay or silent journal deletion. For a provable **NoSpawn** interruption with the original still ready, recovery leaves that original running and thaws it; a subsequent successful fresh Check may offer a new plan without erasing the old journal.

The public v0.2.0/v0.2.1 and historical beta binaries cannot acquire protocol 2 from a check. First adoption requires installing this v0.3.2 source with `--source` or a verified published v0.3.2 release. Old protocol-1 plans and reservations are not silently accepted, rewritten or migrated. A source manifest does not publish a release or switch channels. See [updater status/recovery commands](docs/cli.md#app-update-status-and-recovery).

### Historical local-preview provenance

The standalone/remote-only watcher, independent visibility, compact observation settings, and contextual Settings/conditional recovery originated in four commits (`078f5c2`, `e828c77`, `f638f0c`, `bf23ced`) of the old local `worktree/rapid-harbor-d2a6` checkout. Later menu-bar mode/image and worktree-removal additions were not in `bf23ced`. Those features shipped in beta3 and are now part of stable v0.2.0 `main`; no special local branch or local-only fetch instructions are needed. A historical checkout at `bf23ced` alone still has conditional recovery and lacks later additions. See [historical preview provenance](docs/migrations/unreleased.md).

### Shared installer behavior

`bash scripts/install.sh --prebuilt` downloads the version pinned by this checkout's `herdr-plugin.toml` anonymously over HTTPS with `curl` (no GitHub login) from the checkout's git `origin` repository (falling back to `HERDR_PET_REPOSITORY`, then `hanbong5938/herdr-desktop-pet`). It validates SHA-256, archive paths and entry types, arm64 architecture, code signature, and default character, and **never** falls back to a source build. Running the prebuilt app needs no source-build toolchain; client-attach auto-start still requires the patched host. Optionless installation uses the pinned prebuilt and falls back to source only if it is unavailable or invalid; explicit `--source` builds local changes. App bundles are **ad-hoc signed, not Developer ID signed or notarized**. Installer `curl`, git, and Homebrew formula downloads set no quarantine attribute; a browser-downloaded archive may still require explicit approval to open the app.

Version-specific changes and historical artwork limitations belong in the [release notes](docs/releases/README.md), not this live usage guide. For the old multiline composer’s Enter/Escape changes, see [v0.1.4 → v0.1.6](docs/migrations/v0.1.4-to-v0.1.6.md).

## Controls

These controls describe the v0.3.2 app; the updated menu-bar click and bubble resizing do not retroactively change the published v0.2.0/v0.2.1 binaries. Inline replies first shipped in v0.1.6. **Close Bubble Window** hides only the bubble, keeps selection/drafts for this run, and is disabled/rechecked during IME composition. Character and bubble visibility are independent; hiding the character can leave a standalone bubble.

| Interaction | Effect |
| --- | --- |
| Left-click the menu-bar icon | Open the existing three-tab settings panel directly (character selection and dialogue editing, observation sources, bubble and lifecycle settings, UI language); clicking again brings the same panel forward without closing it or resetting its selected tab. Keyboard activation/AXPress also opens settings |
| Right-click or Control-click the menu-bar icon | Open the native **Show and Enable Character** (when recovery is needed), **Settings…**, **Quit** menu; the settings panel hides before this menu opens |
| Tap the head or body | Trigger a reaction |
| Move back and forth on the head | Pet the character |
| Drag the character body or background | Move the character |
| Option-drag the character window | Move the character |
| Character bottom-right grip | Resize the character |
| Drag the bubble background | Move the character when attached; move only the bubble when standalone |
| Bubble body edges and corners (v0.3.2) | Drag an edge to resize one axis or a corner to resize both, including with Option held |
| Expanded bubble session card (v0.1.6+) | Open a one-line reply beneath the selected local agent session; switching cards waits for active IME composition to end, while remote cards show read-only feedback |
| Inline reply (v0.1.6+) | Enter, Command+Enter, or Send submits to the selected local session; IME composition does not submit |
| Escape in the reply / click outside the bubble (v0.1.6+) | Escape cancels active IME composition first; otherwise either folds only the reply field. Inside-bubble clicks do not automatically fold it; outside clicks do not fold during composition |
| Right-click / Control-click the bubble background or a card header | **Close Bubble Window** hides only the bubble; **Restore automatic size** clears both compact and expanded size overrides |
| Full-window click-through | Pass clicks through both the pet and bubble windows, disabling their interaction |
| Alpha click-through | Pass clicks through transparent character artwork regions only; the bubble remains interactive |

Compact and expanded bubble sizes are saved separately and shared between attached and standalone bubbles. Edge/corner resizing changes neither character scale nor font size. The opposite edge or corner stays fixed through preview and completion on the display where the drag began; subsequent ordinary attached layout continues to follow the character. An edge changes the request on one axis, but newly wrapped content can raise the displayed minimum on the other axis. Height is measured at the actual screen/anchor-limited width without lowering the intrinsic width minimum used for saving. Moving only along an edge's inactive axis, or returning its active displacement to zero, leaves the previous size request unchanged; completion still reflows to the latest automatic size or manual minimum, allowing growth and shrinkage. Expanded messages and cards use their natural heights within one shared panel budget: reserve the card/reply minimum and a message line, then share remaining space without starving either area. Automatic panels retain their overall height limit; manually requested sizes are not subject to that automatic limit. Native scrollbars autohide when content fits the actual viewport, while genuine overflow stays scrollable. A smaller screen limits the displayed size without discarding the requested size; it can recover when enough screen space is available.

Outside an active captured gesture, layout refreshes the resize cursor only when the bubble is the actual pointer target. Native text and controls keep their cursor ownership; reflow does not reset the cursor over the character or another window. A zero-displacement completion saves a changed actual standalone origin with the unchanged size request; an unchanged request and geometry cause no preferences write.

IME composition already in progress blocks starting a resize or choosing **Restore automatic size**. If composition begins after pointer-down, preview pauses; release or native cancellation clears pointer tracking but defers saving and applying pending updates until composition is safe and tracking ends. Displayed cards and reply drafts remain in place meanwhile. The final size candidate uses the latest content, locale, layout, and screen clamp, and saves only once when the gesture completes. If saving fails, previous requested sizes, the prior actual standalone origin, and pending state return without a later ghost save; newer content may still reflow the displayed minimum rather than restoring an exact old pixel size. **Restore automatic size** keeps positions, visibility, the current mode, and reply drafts; position and color resets leave size overrides intact.

The bubble context menu preserves card selection and drafts when canceled. Text areas keep their native copy/paste menus, and buttons, dropdowns, and scrollbars keep their native behavior. **Close Bubble Window** is disabled during active IME composition; finish composition and open the menu again to close. Reopen from **Bubble → Bubble visible** in the menu-bar panel. Closing uses the existing saved bubble visibility setting; reply drafts remain in memory for the current app run.

The menu-bar icon does not open the settings panel directly while the reply contains active IME marked text; finish composition first. Escape or a click outside the settings panel still closes it.

### Independent visibility and menu-bar recovery

Stable v0.2.0 includes the independent visibility and contextual entry points. Right-click/Control-click the character or bubble background/card header and choose **Settings…** to open the full three-tab panel; activation is requested, not guaranteed. The bubble menu offers **Show Character** when hidden. Preserve your settings and managed icon files as described in the [upgrade guide](docs/migrations/v0.1.11-to-v0.2.0.md).

Character visibility (`show`/`hide`/`toggle`) and **Bubble → Bubble visible** (`show_bubble`/`hide_bubble`) are independent:

| Character visible | Bubble visible | Result |
| --- | --- | --- |
| Yes | Yes | Bubble attached to the character |
| Yes | No | Character only |
| No | Yes | Tailless, movable standalone bubble; session cards and replies still work |
| No | No | Neither window shown; app and Herdr session remain running; the menu-bar icon is available for recovery in either v0.2.0 mode |

Hiding the character does not change the saved bubble visibility preference. On first detachment without a saved origin, the standalone bubble starts where the **actually applied attached bubble body** was, not at a placement computed but never displayed. Its origin is saved separately from the character's position; a saved origin wins on later detachments, including while marked text is active. Drag its background to move only the bubble. Showing the character reattaches the visible bubble; hiding it again restores the standalone origin. Compact/expanded changes, replies, and remeasurement keep that origin unless screen clamping is needed; display removal/layout changes recover an offscreen bubble onto a remaining screen. Card selection, view/reply state, and drafts survive character hide/show within the running app; drafts are not restored after restart. Placement changes while standalone apply on reattachment without shifting the standalone bubble. **Reset** clears the old standalone position and reseeds it from the newly applied attached layout when appropriate, without changing visibility or copying the character position.

If the character starts hidden without a saved standalone origin, the first position comes from normal attached layout as a fallback. While the reply contains marked text, hide/show/placement/reset postpone standalone-origin commitment until composition ends; a later reset supersedes an older saved origin. The actual composer callbacks retained its marked text, committed draft, and selection across the observed hide/show/reset transitions; physical IME and keyboard focus were not established.

The bubble context menu preserves card selection and drafts when canceled. Text areas keep their native copy/paste menus, and buttons, dropdowns, and scrollbars keep their native behavior. **Settings…** and **Close Bubble Window** are disabled during active IME composition and rechecked when chosen; finish composition and open the menu again to use them. Reopen a closed bubble through **Bubble → Bubble visible** in the settings panel, opened from the character menu, the standalone bubble's background/card-header menu, or by left-clicking the visible menu-bar icon. If both windows are hidden, left-click the menu-bar icon to open settings directly, or right-click/Control-click it and choose **Settings…** or **Show and Enable Character**. Closing uses the existing saved bubble visibility setting; reply drafts remain in memory for the current app run.

In the original `bf23ced` local preview, the recovery icon was conditional; **stable v0.2.0 defaults to Always show**. Choose **Settings → Menu bar icon**: **Always show** or **Only when recovery is needed**. This choice is independent of character/bubble visibility.

| Stable v0.2.0 state | Always show | Only when recovery is needed |
| --- | --- | --- |
| Character only, standalone bubble only, or both visible and interactive | Icon visible | No icon |
| Both character and bubble hidden | Icon visible | Icon visible |
| Full-window click-through on (regardless of visibility) | Icon visible | Icon visible |
| Alpha click-through only | Icon visible | No icon unless both windows are hidden |

The default status icon is a template pawprint; a chosen image replaces its artwork without changing the status item. In this checkout, left-click opens the existing three-tab panel directly; right-click/Control-click opens its native menu with **Settings…** and **Quit**, plus **Show and Enable Character** only when recovery is needed (both windows hidden or full-window click-through on). Recovery shows the character and disables full-window click-through, preserving bubble visibility and alpha click-through; a visible bubble reattaches, while a hidden bubble stays hidden. **Always show** keeps the icon after recovery; **Only when recovery is needed** removes it after the menu closes. While the app is shutting down, neither mode shows an icon. Alpha-mask pointer polling by itself does not change the recovery condition. CLI `show` still displays only the character, without turning on the bubble.

The native `preferences.json` key `menu_bar_mode` stores `always` or `recovery_only`. New profiles and older profiles without this key (including those from the original conditional-icon preview) use **Always show**; this migration does not infer a saved preference for the old conditional behavior. A successfully saved choice applies immediately and survives restart and unrelated preference saves, preserving unknown preference fields. If saving fails, the previously selected mode and icon remain in effect. Changing the mode does not change window visibility, full-window or alpha click-through, character/bubble positions, reply drafts, or lifecycle settings.

In stable v0.2.0, **Settings → Menu bar icon → Choose image…** imports a static PNG (at most 4 MiB encoded and 1 million decoded pixels; animated, corrupt, truncated, or fully transparent images are rejected); **Restore default icon** selects the template pawprint. The image retains original colors, aspect-fits within 18 pt, and has 1×/2× Retina representations. A successful import stores a managed copy under the configuration directory's `menu-bar-icons/`; the optional `preferences.json` field `menu_bar_icon.asset` points to it, so the external source may be moved or deleted. Back up `menu-bar-icons/` alongside `preferences.json`, `lifecycle.json`, and `characters/`. Missing icon preference uses the default pawprint. Import/save failure leaves the previous choice intact; missing/corrupt managed image on restart shows the default with an error but retains custom metadata. Restoring the default changes the image alone, not mode, visibility, passthrough, geometry, drafts, or lifecycle.

PNG import verifies every chunk CRC (including ancillary chunks and IEND), the IDAT zlib Adler-32 checksum, and the exact inflated scanline length for the image's color format and interlace passes before native decoding. The existing 4 MiB/1-million-pixel limits and supported formats are unchanged. Managed copies are published atomically without replacing an existing file. If an import interrupted on an older working copy left the saved image unusable with a legacy private temporary hardlink, keep the profile and explicitly reimport the **identical valid PNG** through **Choose image…**; this can repair only a confirmed owned, private, same-inode temporary alias. Startup does not sweep old files, and reimport does not repair arbitrary hardlinks, other files, or corrupt images.

Historical beta3 managed-image startup and guarded reset-callback observations are recorded in the [preview evidence](docs/releases/unreleased.md); they are not stable-release proof or physical picker/VoiceOver certification.

### Stable v0.2.0: remove an eligible linked worktree

In the expanded bubble, right-click or Control-click a **card header** and choose **Remove Worktree…** to target that card, even if another card is selected. Right-click or Control-click the **bubble background** to target the selected card as it stood when the menu opened; if no eligible card is selected, the item is absent. The menu identifies the target. Native text-field, button, dropdown, and scrollbar menus/behavior remain unchanged. **Close Bubble Window** is separate: it only hides the bubble and does not remove a checkout, workspace, or session.

Review the confirmation before acting: it names the repository and **exact checkout path** and counts the tabs and panes in the **entire workspace**, not merely the clicked card. Removal closes that workspace and its tabs/panes and terminates their running processes and agents; it deletes the linked checkout, **including ignored files such as build outputs**, and cannot be undone through this app. The Git branch remains; this does not delete the main repository or offer trash/undo. **Cancel** is first and the default (Return, keypad Enter, and Escape cancel); **Remove Worktree** is a separate second action. The native alert explicitly sets Cancel as its default after layout and uses a temporary Escape handler that invokes that same Cancel action; failure to establish the default aborts removal. Check the path and workspace impact carefully before choosing it.

Only a current, live, coherent local source with unambiguous valid linked-worktree metadata is eligible. Main repository roots, remote/retained/offline cards, missing or malformed optional metadata, and older servers that cannot provide the required metadata do not offer deletion. A server that does not support `worktree.remove` reports that limitation instead of falling back to Git, a shell command, SSH, or forced removal. Active IME composition disables removal and is checked again before submission. One operation may be pending at a time; the chosen target is frozen and checked again after confirmation and against a fresh server snapshot. The app sends a single `worktree.remove` request with `force: false`, never auto-trusts a changed target, retries, or optimistically removes the row. The server may reject dirty tracked/untracked files or a locked checkout. Watcher observations remain authoritative on the normal five-second refresh; feedback remains in the bubble even if its row disappears, selection changes, or the window is hidden and reopened. If delivery is uncertain after a write, **check the actual server/worktree state before any manual retry**; the app does not automatically resend.

The request addresses a `workspace_id`, not an expected checkout/generation compare-and-swap: client revalidation is **not an atomic guarantee** against a server restart or workspace rebinding between the final check and removal. Historical beta3 native and backend observations are recorded separately in the [preview evidence](docs/releases/unreleased.md); do not interpret them as a stable v0.2.0 validation or physical-input certification.

### Shared bubble and character settings

Open these settings through the menu-bar panel or contextual **Settings…** entry points in stable v0.2.0.

The menu panel uses native dropdowns for bubble theme and UI language. Choose **System** to follow the system language, or select Korean or English explicitly. Theme and language selections are saved when changed; custom bubble colors keep their **Apply colors** action.

In this **unreleased source checkout**, the Characters tab hides the empty selection/status card when there is no candidate or meaningful feedback. The panel fits the visible character summary; candidates, errors and retained operation outcomes still appear, with an **Operation** title when there is no candidate. Tab switches and live status updates resize the character panel, while Bubble and Settings keep their existing height and scrolling. Small displays clamp the panel and scroll its actual content.

In this **unreleased source checkout**, the **Characters** tab keeps a compact current/candidate/status summary and **Browse Characters…** opens a separate retained, resizable native window. Search names and IDs (and official tags); filter **All**, **Installed**, or **Official Catalog**. Cards show static portraits; browsing does not download pack archives. Closing and reopening the browser preserves its search and filter. Clicking an installed card or historical revision stages a candidate without changing the live character; review it in the fixed browser footer and click **Apply** to select it. **Cancel** clears an unsubmitted selection; closing either window preserves it. Manual **Import** still does not activate a character. Updates, removal, inspection, diagnostics, revision restore, and the separate dialogue editor remain available through the browser. While any pack mutation is pending, selection, Apply, Cancel, Import, Update, Remove, and Restore are blocked; read-only inspection and diagnostics remain available. File-picker and confirmation returns are checked again for busy work and listing changes rather than silently using a newer generation. If an operation is canceled or fails, its candidate remains available. For an unknown submitted result, read the full operation ID in the footer tooltip or diagnostics and run `pack status OPID` instead of retrying blindly. A rejected submission is distinguished from an unknown submitted operation. The latest UI result remains in diagnostics after Apply completes or a new candidate is staged. Successful inactive imports, updates, and removals are complete even when they do not activate a renderer. The panel distinguishes the live character from the committed selection separately from operation completion.
While composing text, stale rendered card actions that would change a character are blocked; **Cancel**, **Inspect**, and **View local pack** remain available. Under temporary pack-store lock contention, portrait previews can remain loading briefly instead of showing an error; actual preview failures still appear as errors.

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

In **v0.3.0**, external dialogue saves do not silently replace a local editor draft. A revision/entry conflict blocks stale Save; **Reload saved** explicitly takes the latest committed text and **Rebase draft** keeps the local text against the new baseline for revalidation before Save. The editor preserves text, selection/focus and undo history while exposing a conflict; active IME marked text defers reload/rebase until composition ends. These conflict controls are absent from prior v0.2.0/v0.2.1 binaries. See the [CLI dialogue contract](docs/cli.md#english).

In **v0.3.1**, external dialogue saves do not silently replace a local editor draft. A revision/entry conflict blocks stale Save; **Reload saved** explicitly takes the latest committed text and **Rebase draft** keeps the local text against the new baseline for revalidation before Save. The editor preserves text, selection/focus and undo history while exposing a conflict; active IME marked text defers reload/rebase until composition ends. These conflict controls are absent from prior v0.2.0/v0.2.1 binaries. See the [CLI dialogue contract](docs/cli.md#english).

In **v0.3.2**, external dialogue saves do not silently replace a local editor draft. A revision/entry conflict blocks stale Save; **Reload saved** explicitly takes the latest committed text and **Rebase draft** keeps the local text against the new baseline for revalidation before Save. The editor preserves text, selection/focus and undo history while exposing a conflict; active IME marked text defers reload/rebase until composition ends. These conflict controls are absent from prior v0.2.0/v0.2.1 binaries. See the [CLI dialogue contract](docs/cli.md#english).

Remote observation first shipped in v0.1.6. In stable v0.2.0 `main`, **Observation sources** uses **This Mac** on and **Remote machines** off by default; the compact layout, contextual Settings, copy-only reconnect UI, and watcher startup correction are included.

1. In an interactive terminal, add and authenticate an SSH machine with Herdr (for example, `herdr machine add workbox`). Follow [Herdr's saved-machine setup guide](https://herdr.dev/docs/0.9.2/connecting-machines/) for SSH access, remote-session selection, and any prompted server setup.
2. Open **Observation sources** through the menu-bar icon or contextual **Settings…**, turn on **Remote machines**, and select the desired enabled machine profiles under **Machines to observe**. Each profile observes one saved remote Herdr session, not every session on that host. The remote host needs a compatible running Herdr server, but does **not** need the desktop-pet app or plugin installed.

**Stable v0.2.0 remote-only startup:**

For a remote-only setup, run the pet on the local Mac, add/authenticate the remote saved machine in a terminal, then turn on **Remote machines** and select that enabled profile in **Observation sources**. Attaching to the remote Herdr session or adding the machine alone does not select it for observation. The remote server does not need desktop-pet installed; the local pet uses the local Herdr CLI to poll the selected profile. **This Mac** can be switched off to exclude local cards, but that switch does not disable a registered local endpoint or change lifecycle shutdown. If there is no healthy local server yet, optionally run `herdr-desktop-pet settings set exit_with_herdr off` **before starting the pet for the first time** to keep it alive while configuring the remote profile; once remote polling is healthy, turn it back on if desired. Use the installed native CLI (or the packaged executable below) for that setting; no Herdr plugin action or remote plugin installation is required.

The pet refreshes Herdr's saved-machine catalog every 5 seconds, even when Remote is off, and polls only selected, enabled profiles every 5 seconds when Remote is on. It remembers profile selections across app restarts and when Remote is switched off; renaming a profile keeps its selection. Disabling or removing a saved profile excludes it from observation without stopping remote sessions or processes. Local and each remote profile have separate source labels on session cards, so matching session IDs on different servers remain distinct.

For compact settings, open the full panel from the character/bubble **Settings…** menu or menu-bar icon (always visible by default, optional recovery-only policy), then use **This Mac** / **Remote machines** and select profiles under **Machines to observe**.

Each machine row separates its name, saved session, and observation state. Long names and diagnostics wrap, and the card and scroll area grow with their contents. Settings distinguish the initial catalog lookup from a confirmed empty list. A failed refresh retains the previous list with an explicit stale-list warning. Switching remote observation off shows **Observation off**, not a previous successful connection status. **How to register** explains the manual terminal setup.
Controls are reused by opaque profile ID so polling/renaming preserve focus and scroll; scrolling is clamped if the list shrinks. The layout separates right-aligned native source switches and nests profiles under **Machines to observe**, avoiding fixed notice gaps and duplicate help. It distinguishes no selection, paused, checking, observing, unavailable, and disabled states as well as catalog lookup/empty. Labels and contextual accessibility names are localized in English and Korean; registration help and folded diagnostics are read-only.

The **This Mac**/**Remote machines** switches and profile selections determine which sources contribute to cards, counts, overall phase, and completion/outcome reactions. The card status filter only narrows which included cards are displayed; it does not change observed sources. Turning off every source shows **Unknown**, not a disconnected warning for a source deliberately excluded.

Remote snapshots are polled rather than streamed: transitions shorter than the 5-second interval can be missed, including brief completions. If a selected machine becomes unavailable, its last observed cards are marked **Offline**, and retained sessions count as unknown rather than live; check its status in **Observation sources**. After reconnection or reselection, the first fresh snapshot is a baseline and does not replay old completion reactions. For SSH authentication recovery, run `herdr machine reconnect '<profile-id>'` yourself in an interactive terminal; find the ID with `herdr machine list`. **Show error details** expands read-only diagnostics; **Copy reconnect command** copies a safely quoted command with native feedback but never runs it. The app does not answer credentials, install remote software, or fall back to local polling for a failed remote profile.

Local observation still supports Herdr 0.9.0 or later. Remote observation additionally needs a local Herdr CLI supporting [saved-machine API forwarding](https://herdr.dev/docs/0.9.2/cli-reference/#saved-ssh-machines) (`herdr --machine <profile-id> api snapshot`) and a compatible running remote Herdr server; Herdr 0.9.2 documents that CLI capability. A missing or incompatible CLI is reported in settings rather than changing the local observation minimum requirement. **Lifecycle client-attach auto-start has the separate patched-host requirement above.**

## Lifecycle settings

The Settings tab has two independent switches: **Auto-start when Herdr starts** (`auto_start`) and **Quit when all Herdr servers disconnect** (`exit_with_herdr`). Both default to on for a new profile. Existing `lifecycle.json` profiles migrate legacy boolean `enabled` to `auto_start` (including `false`), and default missing `exit_with_herdr` to off to preserve their prior independent-running behavior. Both settings live only in `lifecycle.json`, not `preferences.json`; existing endpoints, startup metadata, and unrelated fields are preserved. Invalid values/JSON produce an error instead of being replaced with defaults; a failed save leaves the previous switches and running policy unchanged and shows an error.

`auto_start` on starts an absent pet at server startup or a successful shell/terminal client attach. Automatic `ensure` with auto-start off successfully skips an absent pet; it may register an endpoint for an already-running pet but never forces a hidden pet visible. Manual `start` works with auto-start off and does not change either setting. `stop` and the native **Quit** end only this run (including a pending startup); `restart` starts manually again and preserves settings. If auto-start remains on, the next qualifying server startup/client attach can start the pet again. Turn off `auto_start` to disable future automatic starts. Herdr plugin disable/unlink is a separate host action, not a pet setting; disabled plugins do not receive actions/hooks, and confirmed disable on all registered endpoints terminates an already-running pet immediately.

**Stable v0.2.0 watcher startup correction:**

A valid `plugin_list` response with no `desktop-pet` entry on a newly observed local endpoint means **not installed here**, not disabled: the pet can still connect, read local snapshots, and continue observing remote profiles. An explicit `desktop-pet` entry with `enabled: false` is confirmed disable and detaches that local endpoint. If this pet daemon has already seen an enabled **or disabled** desktop-pet entry at an endpoint, a later missing entry means observed unlink and detaches it as well. That history survives rechecks and reconnects during the same daemon run, but a new daemon starts with no such history. When **all registered local endpoints** are confirmed disabled/unlinked, the pet quits immediately even if a remote profile is healthy or `exit_with_herdr` is off; initial missing is not a confirmed disable/unlink. Otherwise the healthy-connection grace described below applies.

**Lifecycle policy:**

When `exit_with_herdr` is on, the pet quits 30 seconds after the last **healthy server connection** disappears unless a healthy connection returns. An initial launch with no reachable server gets the same finite 30-second grace; one remaining healthy server keeps it alive. Client detach alone is not server loss; stale saved endpoints and failed retries do not extend grace. Switching exit off cancels a live countdown; switching it on while disconnected starts a fresh 30 seconds. With exit off, a manually started pet can continue without Herdr. Changing auto-start does not itself stop or show the current pet. `show`, `hide`, and `toggle` change visibility only; `restart` preserves position, scale, and preferences.

## Plugin actions

```sh
herdr plugin action invoke status --plugin desktop-pet
herdr plugin action invoke settings --plugin desktop-pet
herdr plugin action invoke restart --plugin desktop-pet
herdr plugin action invoke stop --plugin desktop-pet
```

The global `settings` action opens only the lifecycle settings window even with the pet stopped, but requires a running, enabled Herdr host. For offline access, use the executable installed from this source checkout directly (no running pet or enabled/running Herdr host needed):

```sh
PET="./dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet"
"$PET" settings
"$PET" settings get
"$PET" settings set auto_start off
"$PET" settings set exit_with_herdr on
```

`settings get`/`set` return structured JSON; `set` accepts only `auto_start` or `exit_with_herdr` and exactly `on` or `off`, changes no other setting and does not launch the pet. Invalid keys/values, malformed arguments, and save failures return errors rather than silently resetting settings. `status` reports `auto_start` and `exit_with_herdr`. Additional action IDs: lifecycle `start`, `ensure`, `stop`, `restart`, `status`, `settings`; visibility `show`, `hide`, `toggle`; click-through `passthrough`, `alpha_passthrough`; bubble `show_bubble`, `hide_bubble`, `bubble_above`, `bubble_below`, `bubble_left`, `bubble_right`, `bubble_auto`; geometry `reset`, `bigger`, `smaller`.


### Native automation CLI (v0.3.0)

### Native automation CLI (v0.3.1)

### Native automation CLI (v0.3.2)
| --- | --- |
Use the executable built with `bash scripts/install.sh --source`, or a confirmed v0.3.2 prebuilt; `herdr plugin action invoke` remains the separate Herdr lifecycle interface. Native `herdr-desktop-pet --help` lists actual options. `presentation get/set/reset/status` controls absolute visibility, placement and scale; `preferences get/set/status` controls saved/effective settings; `sessions list/show/prompt/status` reads observed local sessions and submits only to a specifically identified local agent; `dialogue list/get/set/reset-entry/reset-character/status` edits per-target overrides; `worktree inspect/remove/status` requires an expiring one-use inspect token before irreversible removal. There is no public `orchestrate` command: automation uses these explicit families.
| `preferences get`; `preferences set --menu-bar always --expected-revision N --operation-id OPID` | Menu-bar mode and other preference fields use this prepared-v0.3.0 command; `preferences status --instance ID --operation-id OPID` is family-scoped. CLI `--language` accepts exactly `system|ko|en` before storage access; repeated `--machine ID` replaces the **whole** list and rejects unknown/disabled IDs. GUI may retain/remove previously saved unavailable IDs while adding only currently enabled IDs. |
| `sessions list --instance ID --limit 128`; `sessions show --instance ID --source N --generation N --terminal TERMINAL_ID` | 1–128 is a **page size**, not a total row cap. Each response page fits a 512 KiB encoded frame including JSON escapes, cursor and newline; a smaller prefix is returned when necessary, and the CLI collects all coherent pages. `sessions prompt` requires the same four identity flags and exactly one of `--text`, `--file`, `--stdin`; an ACK is not agent completion. |
| `preferences get`; `preferences set --menu-bar always --expected-revision N --operation-id OPID` | Menu-bar mode and other preference fields use this prepared-v0.3.1 command; `preferences status --instance ID --operation-id OPID` is family-scoped. CLI `--language` accepts exactly `system|ko|en` before storage access; repeated `--machine ID` replaces the **whole** list and rejects unknown/disabled IDs. GUI may retain/remove previously saved unavailable IDs while adding only currently enabled IDs. |
| `worktree inspect --instance ID --source N --generation N --terminal TERMINAL_ID` | Review the returned frozen checkout/workspace and token before separately running `worktree remove --token TOKEN`; no path, force, trash or undo. Ignored files can be deleted even with backend `force: false`. Never use a live user's checkout as a trial target. |
| `preferences get`; `preferences set --menu-bar always --expected-revision N --operation-id OPID` | Menu-bar mode and other preference fields use this prepared-v0.3.2 command; `preferences status --instance ID --operation-id OPID` is family-scoped. CLI `--language` accepts exactly `system|ko|en` before storage access; repeated `--machine ID` replaces the **whole** list and rejects unknown/disabled IDs. GUI may retain/remove previously saved unavailable IDs while adding only currently enabled IDs. |
Domain mutations can choose `--operation-id OPID` before sending and `--wait SECONDS` (finite, at most 86400; presentation also allows zero, other domain families require positive) or `--no-wait`; default wait is 15 seconds. After submission ACK, polling has one absolute wait deadline across sleep, status RPC and decoding, including domain reads and partial-color reads; it cannot restart a full poll timeout or treat a late result as success. A deadline does not cancel, roll back or resend; an early transport failure can instead leave delivery uncertain. Investigate with the matching family `status --instance ID --operation-id OPID`; presentation and pack status instead take positional `OPID`. An expired/unknown status after daemon restart is **not proof** the original mutation never ran. Acceptance, committed storage, native application, and agent completion are distinct.

In the dialogue editor, first-ready metadata hydrates an untouched field even while focused; raw premetadata drafts (including whitespace and IME composition) survive, with deferred hydration settled after unmark. Selection preserves the exact authored reference or an unavailable reference and its draft rather than silently switching revisions. Cached metadata errors remain cached on automatic polls; explicit reopen/selection or a new read can retry once. The visible settings panel's quiet color-conflict checks update control eligibility without replacing draft text, focus, selection or undo, and stop when closed. See the [English contract](docs/cli.md#english) / [한국어 계약](docs/cli.md#한국어) for identity JSON, operation states and safe procedures.

## Character packs

In this **unreleased source checkout**, use **Characters → Browse Characters…** in the menu-bar panel, or run the existing native pack CLI from the checkout:

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

For pack commands, `CHARACTER_ID` is positional (`pack select ID`, `pack restore ID --revision N`); there is no `--id` for selection. In **v0.3.0**, pack mutations accept `--operation-id ID`, `--expected-generation N`, and `--wait SECONDS` or `--no-wait`. Default offline mutation completes its worker synchronously; explicit async/finite waits need a running daemon and never secretly start one. After ACK, a finite wait uses one absolute deadline through sleeps, complete status replies and decoding, without resending; a late terminal reply cannot turn an elapsed wait into success. `--no-wait` does not mask a terminal failure in the initial reply: failed/canceled/uncertain terminal states exit nonzero, whereas accepted/pending is only an ACK. Use `pack status OPID` for uncertain outcomes; acceptance is not native application or persistence. Earlier v0.2.0/v0.2.1 binaries do not provide these additional controls.

For pack commands, `CHARACTER_ID` is positional (`pack select ID`, `pack restore ID --revision N`); there is no `--id` for selection. In **v0.3.1**, pack mutations accept `--operation-id ID`, `--expected-generation N`, and `--wait SECONDS` or `--no-wait`. Default offline mutation completes its worker synchronously; explicit async/finite waits need a running daemon and never secretly start one. After ACK, a finite wait uses one absolute deadline through sleeps, complete status replies and decoding, without resending; a late terminal reply cannot turn an elapsed wait into success. `--no-wait` does not mask a terminal failure in the initial reply: failed/canceled/uncertain terminal states exit nonzero, whereas accepted/pending is only an ACK. Use `pack status OPID` for uncertain outcomes; acceptance is not native application or persistence. Earlier v0.2.0/v0.2.1 binaries do not provide these additional controls.

For pack commands, `CHARACTER_ID` is positional (`pack select ID`, `pack restore ID --revision N`); there is no `--id` for selection. In **v0.3.2**, pack mutations accept `--operation-id ID`, `--expected-generation N`, and `--wait SECONDS` or `--no-wait`. Default offline mutation completes its worker synchronously; explicit async/finite waits need a running daemon and never secretly start one. After ACK, a finite wait uses one absolute deadline through sleeps, complete status replies and decoding, without resending; a late terminal reply cannot turn an elapsed wait into success. `--no-wait` does not mask a terminal failure in the initial reply: failed/canceled/uncertain terminal states exit nonzero, whereas accepted/pending is only an ACK. Use `pack status OPID` for uncertain outcomes; acceptance is not native application or persistence. Earlier v0.2.0/v0.2.1 binaries do not provide these additional controls.
- In the **v0.3.0 browser**, installed cards and historical revisions only stage a candidate until **Apply**. Native CLI `pack select` immediately selects the newest revision, and `pack restore` immediately selects an existing historical revision; these commands do not require UI Apply.
- Removing the active pack switches back to Rubelia.
- In the **v0.3.1 browser**, installed cards and historical revisions only stage a candidate until **Apply**. Native CLI `pack select` immediately selects the newest revision, and `pack restore` immediately selects an existing historical revision; these commands do not require UI Apply.
- Keep operation IDs. If a mutation's outcome is uncertain, run `"$PET" pack status OPERATION_ID` instead of blindly repeating it.
- In the **v0.3.2 browser**, installed cards and historical revisions only stage a candidate until **Apply**. Native CLI `pack select` immediately selects the newest revision, and `pack restore` immediately selects an existing historical revision; these commands do not require UI Apply.
For authoring, see the [character creator guide](.agents/skills/character-creator/SKILL.md) and `tools/character-pack.py`. The packaged app includes them at `Contents/Resources/creator/SKILL.md` and `Contents/Resources/creator/character-pack.py`. From a source checkout, the [local Qwen + See-through production skill](.agents/skills/create-pet-character/SKILL.md) generates ten independent poses from an approved illustration; its external engines and model weights are not bundled. Rubelia is the bundled rig template. PNG templates, Aurora, Coding Cat, optional Rubelia outfits, and their artwork generators are maintained in [herdr-characters](https://github.com/hanbong5938/herdr-characters); use `--templates-root PATH` to supply an external template.

## Configuration and troubleshooting

- `HERDR_PLUGIN_CONFIG_DIR`: `lifecycle.json` (both lifecycle switches), `preferences.json` (other preferences), and managed characters.
- `HERDR_PLUGIN_STATE_DIR`: process lock, control socket, and daemon log (`desktop-pet.log`).
- Without injected paths, the app uses Herdr's XDG config/state plugin directories.
- Native commands accept `--config-dir` and `--state-dir` for isolated profiles; injected environment paths take precedence.
- In stable v0.2.0, a standalone bubble's background/card-header menu offers **Show Character**. With both windows hidden or full-window click-through on, use the menu-bar icon's **Show and Enable Character** in either icon mode. **Always show** keeps the icon available in interactive states; recovery-only hides it there. These visibility states do not mean startup failed. If the daemon is stopped or did not start after linking, check unknown-hook warnings, invoke `start`, and inspect `status` and the daemon log. An off `auto_start` is an intentional automatic skip.
- If `start` reports `Herdr watchers are shutting down`, check which Herdr socket the CLI inherited (`HERDR_SOCKET_PATH` / `HERDR_CLIENT_SOCKET_PATH`), which CLI executable/version is on `PATH`, and which Herdr server actually owns that socket. A `herdr plugin list` against another server does not diagnose the daemon's observed endpoint. Use the intended CLI and socket/session together; for remote errors, also check the saved-machine forwarding CLI version and the selected profile. A patched Herdr executable matters for `client.attached` auto-start, but does not install desktop-pet on any server.
- Run the app on the Mac whose desktop you intend to display: launching it over SSH on a remote host does not display its windows on your local Mac.
- For remote observation errors, check **Observation sources** and the saved profile's status; see [Observation sources](#observation-sources) for authentication recovery and CLI compatibility.
- If a release download fails, check network access to github.com. `--prebuilt` does not fall back; the default mode falls back to a source build, which requires the source toolchain.
- Use Herdr's upstream provider integrations. Restart/reload existing provider sessions after installing their upstream lifecycle hooks.

## Development

After a source installation:

Native hit-overlay blending uses typed subexpressions for compatibility with the release runner’s Xcode 16.4 Swift compiler. Keep those type boundaries when modifying the overlay; the source toolchain still requires Xcode Command Line Tools.

Debug and release native builds require `codesign` and ad-hoc sign only `rig-decode-worker` with `com.apple.security.cs.allow-jit=true`, using `native/rig/RigDecodeWorker.entitlements.plist`. Cargo tracks plist changes so they rebuild and re-sign the worker. JavaScriptCore can then JIT-compile the trusted decoder bundle; the main app and native rig library receive no JIT entitlement, and worker isolation, validation, and resource limits stay unchanged.

`bun run package:native` reapplies that worker-only entitlement with the selected signing identity, then verifies the effective entitlements and strict worker/app signatures after the outer app is signed. Signing or verification failure stops packaging. The default ad-hoc package is for local use, not Developer ID signing or notarization.

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

The current [source record](assets/rubelia-default/source-record.json) and [background-repair record](assets/rubelia-default/background-repair.json) remain the artwork authorities. Artwork-owner approval does not grant model rights; software MIT does not relicense artwork. Qwen’s research license is retained as an authoring record, and model weights/engines are not bundled. Version-specific redraws, face corrections, and historical limitations are recorded in [v0.1.6](docs/releases/v0.1.6.md), [v0.1.9](docs/releases/v0.1.9.md), and [v0.1.10](docs/releases/v0.1.10.md).

