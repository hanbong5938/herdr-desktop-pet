# Herdr Desktop Pet

A native macOS desktop companion for [Herdr](https://herdr.dev). Rubelia lives on your desktop, reacts to touch, displays observed Herdr session activity, and lets you submit text to a selected agent session from its status bubble.

**English** · [한국어](readme.ko.md) · [Version documentation](docs/releases/README.md) · [Releases](https://github.com/hanbong5938/herdr-desktop-pet/releases)

> **Public `main` source builds and the pinned public app v0.1.11 use the baseline menu-bar UI described below.** The four pending native commits are local-branch previews only: they are **not merged into `main` and not included in any public v0.1.11 binary**. The menu-bar additions and worktree removal described as current-working-copy features below are later, uncommitted changes, not part of those four commits. `--source` builds the checkout you already have; it does not obtain these changes. See [v0.1.11 notes](docs/releases/v0.1.11.md), [local pending notes](docs/releases/unreleased.md), and the [local-checkout guide](docs/migrations/unreleased.md) ([한국어](docs/migrations/unreleased.ko.md)).

The published `beta/0.2.0-beta.2` remains separate from public `main` and stable v0.1.11. This working copy also supplies the reply clipboard fix selected for `beta/0.2.0-beta.3`; until beta3 publication and verification, the rolling beta formula still installs beta2. “Current working copy only” features are not in the stable installer or Homebrew app.

<img src="assets/rubelia-thumbnail.png" alt="Rubelia, the default desktop companion" width="220">

## Features

- Native desktop character with a menu-bar control panel.
- Observe local Herdr sessions by default, with optional saved-machine remote observation.
- Session-aware states: idle, running, waiting, and unknown.
- Readable session cards with task titles, workspace/tab context, source labels, and a separate status label.
- Inline replies beneath selected local agent session cards in the expanded status bubble, shipped since v0.1.6; remote cards remain read-only. See the [v0.1.4 → v0.1.6 migration](docs/migrations/v0.1.4-to-v0.1.6.md).
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
| App | Pinned prebuilt v0.1.11 (downloaded by the plugin installer or Homebrew; no build toolchain), or build this checkout with Rust/Cargo, Bun, Node.js/npm, and Xcode Command Line Tools |

**The patch is REQUIRED for client-attach auto-start.** Stock Herdr 0.9.0 and 0.9.3 do not provide `client.attached`; a manifest version floor is not proof of hook support. Apply the supplied patch to official 0.9.3 source and build/run that host following its build instructions; inspect plugin link warnings and verify the `client.attached` subscription is accepted. Do not treat an unknown-hook warning as success. Intel Macs, Windows, and Linux are not supported by the native distribution.

## Install as a Herdr plugin (recommended)

Install from the public [plugin repository](https://github.com/hanbong5938/herdr-desktop-pet) like any other Herdr plugin:

```sh
herdr plugin install hanbong5938/herdr-desktop-pet
herdr plugin action invoke start --plugin desktop-pet
```

Herdr clones the repository into its plugin directory and runs the manifest's `[[build]]` step, `bash scripts/install.sh`. The installer downloads the release pinned to the manifest `version` (v0.1.11) anonymously over HTTPS with `curl`, verifies its SHA-256, archive paths and entry types, arm64 architecture, code signature, and default character, and installs it into the plugin directory's `dist/`. No GitHub login or build toolchain is needed in the normal case, and nothing is installed outside the plugin directory. If the pinned prebuilt is unavailable or fails verification, the installer prints a notice and falls back to a source build, which requires the source toolchain listed above. The app is **ad-hoc signed, not notarized**; because it is fetched by `curl` rather than a browser or cask download, no quarantine attribute is set and macOS shows no Gatekeeper prompt. Client-attach auto-start still needs the supplied host patch: run these commands against the patched Herdr host described above.

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

## Opt-in beta UI test (not the stable install)

For Apple Silicon on macOS 13+, [`beta/0.2.0-beta.3`](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/beta%2F0.2.0-beta.3) is the planned opt-in test release; use its download only after publication and verification. The separate rolling beta Homebrew formula still installs 0.2.0-beta.2 until it is updated after that release. Neither channel changes public `main`, stable v0.1.11, the manifest, `--prebuilt`, the stable Homebrew formula, or normal plugin install. Immutable beta1 has reply clipping; beta2 fixes clipping and the malformed-link scanner but lacks reply Command-A/C/X/V clipboard dispatch. Selected beta3 source adds that native clipboard fix. Physical keyboard/IME verification remains outstanding.

```sh
brew install hanbong5938/tap/herdr-desktop-pet-beta
herdr-desktop-pet-beta --version
herdr-desktop-pet-beta start
herdr-desktop-pet-beta status
```

The beta formula installs `HerdrDesktopPetBeta.app` inside Homebrew's prefix, **not** `/Applications`; `herdr-desktop-pet-beta` is separate from the stable CLI. It currently reports `0.2.0-beta.2`; after the verified beta3 formula update it should report `0.2.0-beta.3`. New users can use the install command above; existing beta users can run `brew update && brew upgrade herdr-desktop-pet-beta` **after** that update (not upgrade the stable formula). Remove only beta with `brew uninstall herdr-desktop-pet-beta`. After beta3 assets are published, manually download `HerdrDesktopPet-v0.2.0-beta.3-macos-arm64.tar.gz` and its `.sha256` (or `SHA256SUMS`) from the linked beta3 release and verify SHA-256 before unpacking:

```sh
mkdir -p "$HOME/HerdrBeta"
tar -xzf "$HOME/Downloads/HerdrDesktopPet-v0.2.0-beta.3-macos-arm64.tar.gz" -C "$HOME/HerdrBeta"
BETA="$HOME/HerdrBeta/HerdrDesktopPetBeta.app/Contents/MacOS/herdr-desktop-pet"
"$BETA" --version
"$BETA" start
```

Adjust the download path if necessary. Before either beta `start`, quit the running stable pet and back up its configuration and state, including `preferences.json`, `lifecycle.json`, `characters/`, and managed `menu-bar-icons/`. Unless explicitly using isolated config **and** state directories, beta and stable share the same profile and control namespace: do not run them concurrently. The ad-hoc-signed, non-notarized manual download may require explicit macOS Gatekeeper approval. Only the isolated beta source snapshot changes version; this working copy remains v0.1.11. Follow the [manual UI checklist](docs/migrations/unreleased.md#optional-manual-beta-test) before testing deletion on a disposable linked worktree.

Historical beta2 proof only (not beta3 verification): the public beta2 prerelease points to `9035c11e5acf76a1092ddf3a08d4360eac680495`; anonymous downloads confirmed its archive SHA-256 `ddb61fa2ff45aa0fcc603c175423667512b7fa043461b6ab5e9f2c37576a6432` against published checksums and GitHub digest, and its extracted app passed strict signature/version/arm64 checks. Isolated beta1 → beta2 upgrade and fresh beta2 install/test passed; extracted and upgraded beta2 reached UI/control/registration/data readiness against stock Herdr 0.9.3 with separate config and state. These are not beta3 publication/install proof or physical input, picker, alert-key, pointer, or VoiceOver verification.

## Install this checkout from source

For development checkouts, `plugin link` does not run the manifest's `[[build]]`, so build the checkout yourself. Building public `main` provides the baseline UI, not the local pending previews. For client-attach auto-start, run the patched Herdr host in an isolated profile (follow the [patched-host deployment guide](integrations/herdr/README.md) and use that binary explicitly, rather than an unpatched `herdr` on `PATH`):

```sh
bash scripts/install.sh --source
"$PATCHED_HERDR" plugin link "$PWD" --enabled
"$PATCHED_HERDR" plugin action invoke start --plugin desktop-pet
```

The installer installs pinned JavaScript dependencies, builds the Rust executable, packages the native rig runtime and creator resources, and validates the app. Linking/enabling on an already-running server does not itself launch the pet: invoke `start` once, or wait for a subsequent successful shell/terminal client attach when `auto_start` is on. Server startup also runs automatic `ensure`. Inspect link warnings: an unknown `client.attached` hook means the host lacks the required patch. Herdr plugin actions require a running, enabled host; direct native settings commands below work without one.

### Local pending preview checkout only

Preview guidance here applies only if you **already have the appropriate local checkout** of `worktree/rapid-harbor-d2a6` at `bf23ced1649fd0074aec8736644c8f02aa0c492b`, containing these four unmerged commits:

| Local commit | Pending change |
| --- | --- |
| `078f5c2` | Standalone/remote-only watcher startup |
| `e828c77` | Independent character/bubble visibility |
| `f638f0c` | Compact observation settings |
| `bf23ced` | Contextual Settings and conditional recovery |

This branch and these commits are local provenance, **not public checkout/download links**; their availability elsewhere is not guaranteed. They remain unmerged into `main`, and no public v0.1.11 binary contains them. Only from that existing local checkout, use the source-install commands above with your isolated patched host. Use explicit `--source` for the preview: `--prebuilt` never includes it, and optionless installation normally selects the pinned public binary (source fallback builds only the checkout already present). Do not use a public `main` checkout as a substitute.

The four commits above describe the original local preview, whose status icon was conditional. The **selectable menu-bar policy, custom icon image, and worktree removal described below are later additions in this working copy**, not changes contained in `bf23ced` or stable v0.1.11; they are included in the published beta2 snapshot. A checkout at that historical commit alone retains its original conditional recovery behavior and does not provide worktree removal.

### Shared installer behavior

`bash scripts/install.sh --prebuilt` downloads the version pinned by this checkout's `herdr-plugin.toml` anonymously over HTTPS with `curl` (no GitHub login) from the checkout's git `origin` repository (falling back to `HERDR_PET_REPOSITORY`, then `hanbong5938/herdr-desktop-pet`). It validates SHA-256, archive paths and entry types, arm64 architecture, code signature, and default character, and **never** falls back to a source build. Running the prebuilt app needs no source-build toolchain; client-attach auto-start still requires the patched host. Optionless installation uses the pinned prebuilt and falls back to source only if it is unavailable or invalid; explicit `--source` builds local changes. App bundles are **ad-hoc signed, not Developer ID signed or notarized**. Installer `curl`, git, and Homebrew formula downloads set no quarantine attribute; a browser-downloaded archive may still require explicit approval to open the app.

Version-specific changes and historical artwork limitations belong in the [release notes](docs/releases/README.md), not this live usage guide. For the old multiline composer’s Enter/Escape changes, see [v0.1.4 → v0.1.6](docs/migrations/v0.1.4-to-v0.1.6.md).

## Controls

These controls apply to public `main` source builds and v0.1.11. Inline replies shipped in v0.1.6. The v0.1.11 bubble context menu provides **Close Bubble Window**; it hides only the bubble, keeps selection/drafts for this run, and is disabled/rechecked during IME composition. Use the menu-bar panel for settings and visibility recovery; hiding the character does not provide a standalone bubble.

| Interaction | Effect |
| --- | --- |
| Menu-bar icon | Character selection and dialogue editing, observation sources, bubble and lifecycle settings, and UI language |
| Tap the head or body | Trigger a reaction |
| Move back and forth on the head | Pet the character |
| Drag the body or background | Move the character |
| Option-drag anywhere | Move the character |
| Bottom-right grip | Resize the character |
| Expanded bubble session card (v0.1.6+) | Open a one-line reply beneath the selected local agent session; switching cards waits for active IME composition to end, while remote cards show read-only feedback |
| Inline reply (v0.1.6+) | Enter, Command+Enter, or Send submits to the selected local session; IME composition does not submit |
| Escape in the reply / click outside the bubble (v0.1.6+) | Escape cancels active IME composition first; otherwise either folds only the reply field. Inside-bubble clicks do not automatically fold it; outside clicks do not fold during composition |
| Right-click / Control-click the bubble background or a card header | **Close Bubble Window** hides only the bubble, not the character, app, or Herdr session |
| Full-window click-through | Pass clicks through both the pet and bubble windows, disabling their interaction |
| Alpha click-through | Pass clicks through transparent character artwork regions only; the bubble remains interactive |

The bubble context menu preserves card selection and drafts when canceled. Text areas keep their native copy/paste menus, and buttons, dropdowns, and scrollbars keep their native behavior. **Close Bubble Window** is disabled during active IME composition; finish composition and open the menu again to close. Reopen from **Bubble → Bubble visible** in the menu-bar panel. Closing uses the existing saved bubble visibility setting; reply drafts remain in memory for the current app run.

### Local checkout: independent visibility and menu-bar recovery

The independent visibility and contextual entry points require the local commits listed above; they are **not in `main` source builds or public v0.1.11**. The selectable menu-bar policy and custom image below additionally require this current working copy, not just the original `bf23ced` checkout. Right-click/Control-click the character or bubble background/card header and choose **Settings…** to open the same full three-tab panel; activation is requested, not guaranteed. The bubble menu also adds **Show Character** when hidden. See [local-checkout and preference-preservation guidance](docs/migrations/unreleased.md).

Character visibility (`show`/`hide`/`toggle`) and **Bubble → Bubble visible** (`show_bubble`/`hide_bubble`) are independent:

| Character visible | Bubble visible | Result |
| --- | --- | --- |
| Yes | Yes | Bubble attached to the character |
| Yes | No | Character only |
| No | Yes | Tailless, movable standalone bubble; session cards and replies still work |
| No | No | Neither window shown; app and Herdr session remain running; the menu-bar icon is available for recovery in either current-working-copy mode |

Hiding the character does not change the saved bubble visibility preference. On first detachment without a saved origin, the standalone bubble starts where the **actually applied attached bubble body** was, not at a placement computed but never displayed. Its origin is saved separately from the character's position; a saved origin wins on later detachments, including while marked text is active. Drag its background to move only the bubble. Showing the character reattaches the visible bubble; hiding it again restores the standalone origin. Compact/expanded changes, replies, and remeasurement keep that origin unless screen clamping is needed; display removal/layout changes recover an offscreen bubble onto a remaining screen. Card selection, view/reply state, and drafts survive character hide/show within the running app; drafts are not restored after restart. Placement changes while standalone apply on reattachment without shifting the standalone bubble. **Reset** clears the old standalone position and reseeds it from the newly applied attached layout when appropriate, without changing visibility or copying the character position.

If the character starts hidden without a saved standalone origin, the first position comes from normal attached layout as a fallback. While the reply contains marked text, hide/show/placement/reset postpone standalone-origin commitment until composition ends; a later reset supersedes an older saved origin. The actual composer callbacks retained its marked text, committed draft, and selection across the observed hide/show/reset transitions; physical IME and keyboard focus were not established.

The bubble context menu preserves card selection and drafts when canceled. Text areas keep their native copy/paste menus, and buttons, dropdowns, and scrollbars keep their native behavior. **Settings…** and **Close Bubble Window** are disabled during active IME composition and rechecked when chosen; finish composition and open the menu again to use them. Reopen a closed bubble through **Bubble → Bubble visible** in the settings panel, opened from the character menu or, when the bubble is standalone, its background/card-header menu. If both windows are hidden, use the menu-bar icon's **Settings…** instead. Closing uses the existing saved bubble visibility setting; reply drafts remain in memory for the current app run.

In the original `bf23ced` preview, the recovery icon was conditional: it appeared only with both windows hidden or full-window click-through on, and interactive states occupied no menu-bar slot. **In this current working copy**, choose **Settings → Menu bar icon**: **Always show** (the default) or **Only when recovery is needed**. This choice is independent of the character and bubble visibility settings.

| Current working-copy state | Always show | Only when recovery is needed |
| --- | --- | --- |
| Character only, standalone bubble only, or both visible and interactive | Icon visible | No icon |
| Both character and bubble hidden | Icon visible | Icon visible |
| Full-window click-through on (regardless of visibility) | Icon visible | Icon visible |
| Alpha click-through only | Icon visible | No icon unless both windows are hidden |

The default status icon is a template pawprint; a chosen image replaces its artwork without changing the status item or its menu. In either mode its menu always offers **Settings…** (the same full panel) and **Quit**; **Show and Enable Character** appears only when recovery is needed (both windows hidden or full-window click-through on). Recovery shows the character and disables full-window click-through, preserving bubble visibility and alpha click-through; a visible bubble reattaches, while a hidden bubble stays hidden. **Always show** keeps the icon after recovery; **Only when recovery is needed** removes it after the menu closes. While the app is shutting down, neither mode shows an icon. Alpha-mask pointer polling by itself does not change the recovery condition. CLI `show` still shows only the character, without turning on the bubble.

The native `preferences.json` key `menu_bar_mode` stores `always` or `recovery_only`. New profiles and older profiles without this key (including those from the original conditional-icon preview) use **Always show**; this migration does not infer a saved preference for the old conditional behavior. A successfully saved choice applies immediately and survives restart and unrelated preference saves, preserving unknown preference fields. If saving fails, the previously selected mode and icon remain in effect. Changing the mode does not change window visibility, full-window or alpha click-through, character/bubble positions, reply drafts, or lifecycle settings.

**Current working copy only:** in **Settings → Menu bar icon**, use **Choose image…** to import a static PNG (at most 4 MiB encoded and 1 million decoded pixels; animated, corrupt, truncated, or fully transparent images are rejected), or **Restore default icon** for the template pawprint. The chosen image keeps its original colors, aspect-fits within 18 pt, and has 1×/2× Retina representations. A successful import stores a managed copy under the configuration directory's `menu-bar-icons/`; the optional `preferences.json` field `menu_bar_icon.asset` points to it, so the external source may be moved or deleted. Back up `menu-bar-icons/` alongside `preferences.json`, `lifecycle.json`, and `characters/`. Missing icon preference uses the default pawprint. An import/save failure leaves the previous image and settings intact; a missing or corrupt managed image on restart shows the default with an error while keeping the custom-image preference for recovery. Restoring the default changes only the image, not `menu_bar_mode`, character/bubble visibility, click-through, geometry, drafts, or lifecycle.

PNG import verifies every chunk CRC (including ancillary chunks and IEND), the IDAT zlib Adler-32 checksum, and the exact inflated scanline length for the image's color format and interlace passes before native decoding. The existing 4 MiB/1-million-pixel limits and supported formats are unchanged. Managed copies are published atomically without replacing an existing file. If an import interrupted on an older working copy left the saved image unusable with a legacy private temporary hardlink, keep the profile and explicitly reimport the **identical valid PNG** through **Choose image…**; this can repair only a confirmed owned, private, same-inode temporary alias. Startup does not sweep old files, and reimport does not repair arbitrary hardlinks, other files, or corrupt images.

In an isolated current-binary startup check, an already-installed private managed PNG stayed selected after its external source was deleted and the app restarted. Its real status-button image was non-template, 18 pt, with 18×18 and 36×36 bitmap representations; the saved choice and unknown preferences survived. This seeded-managed-asset check did **not** exercise **Choose image…**, importing, picker cancellation/rejection, or visual/VoiceOver interaction.
After Settings opened through the actual status-menu action, the enabled **Restore default icon** button's AppKit `performClick` ran the production reset callback: the status image became a template, `preferences.json` lost only `menu_bar_icon` (all other known/unknown fields stayed identical), and the old owned managed PNG was unlinked. This was not a physical button click or chooser Cancel/valid/corrupt PNG selection or VoiceOver proof.

### Current working copy only: remove a linked worktree

In the expanded bubble, right-click or Control-click a **card header** and choose **Remove Worktree…** to target that card, even if another card is selected. Right-click or Control-click the **bubble background** to target the selected card as it stood when the menu opened; if no eligible card is selected, the item is absent. The menu identifies the target. Native text-field, button, dropdown, and scrollbar menus/behavior remain unchanged. **Close Bubble Window** is separate: it only hides the bubble and does not remove a checkout, workspace, or session.

Review the confirmation before acting: it names the repository and **exact checkout path** and counts the tabs and panes in the **entire workspace**, not merely the clicked card. Removal closes that workspace and its tabs/panes and terminates their running processes and agents; it deletes the linked checkout, **including ignored files such as build outputs**, and cannot be undone through this app. The Git branch remains; this does not delete the main repository or offer trash/undo. **Cancel** is first and the default (Return, keypad Enter, and Escape cancel); **Remove Worktree** is a separate second action. The native alert explicitly sets Cancel as its default after layout and uses a temporary Escape handler that invokes that same Cancel action; failure to establish the default aborts removal. Check the path and workspace impact carefully before choosing it.

Only a current, live, coherent local source with unambiguous valid linked-worktree metadata is eligible. Main repository roots, remote/retained/offline cards, missing or malformed optional metadata, and older servers that cannot provide the required metadata do not offer deletion. A server that does not support `worktree.remove` reports that limitation instead of falling back to Git, a shell command, SSH, or forced removal. Active IME composition disables removal and is checked again before submission. One operation may be pending at a time; the chosen target is frozen and checked again after confirmation and against a fresh server snapshot. The app sends a single `worktree.remove` request with `force: false`, never auto-trusts a changed target, retries, or optimistically removes the row. The server may reject dirty tracked/untracked files or a locked checkout. Watcher observations remain authoritative on the normal five-second refresh; feedback remains in the bubble even if its row disappears, selection changes, or the window is hidden and reopened. If delivery is uncertain after a write, **check the actual server/worktree state before any manual retry**; the app does not automatically resend.

The request addresses a `workspace_id`, not an expected checkout/generation compare-and-swap: client revalidation is **not an atomic guarantee** against a server restart or workspace rebinding between the final check and removal. This current-working-copy behavior has native tests and isolated real-backend exercise. A top-layer Window Server shield obstructed app-window clicks and input, so actual alert Return/keypad Enter/Escape, focused Delete, pointer context menus, and physical IME remain unverified; do not treat the alert configuration or callback-level composer observations as keyboard/visual proof.

### Shared bubble and character settings

For public `main` and v0.1.11, open these settings from the menu-bar panel; only the local preview adds the contextual entry points above.

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

Remote observation itself shipped in v0.1.6. Public `main` source builds and v0.1.11 use **Observation sources** in the menu-bar settings, with **Local** on and **Remote** off by default. They do not include the compact layout, contextual Settings entry points, copy-only reconnect UI, or watcher startup correction described in the explicitly local-preview guidance below.

1. In an interactive terminal, add and authenticate an SSH machine with Herdr (for example, `herdr machine add workbox`). Follow [Herdr's saved-machine setup guide](https://herdr.dev/docs/0.9.2/connecting-machines/) for SSH access, remote-session selection, and any prompted server setup.
2. Open the menu-bar panel's **Observation sources** settings, turn on **Remote**, and select the desired enabled machine profiles. Each profile observes its one saved remote Herdr session, not every session on that host. The remote host needs a compatible running Herdr server, but does **not** need the desktop-pet app or plugin installed.

**Local pending preview only — remote-only startup guidance (requires `078f5c2` in the local checkout above; not `main` or public v0.1.11):**

For a remote-only setup, run the pet on the local Mac, add/authenticate the remote saved machine in a terminal, then turn on **Remote machines** and select that enabled profile in **Observation sources**. Attaching to the remote Herdr session or adding the machine alone does not select it for observation. The remote server does not need desktop-pet installed; the local pet uses the local Herdr CLI to poll the selected profile. **This Mac** can be switched off to exclude local cards, but that switch does not disable a registered local endpoint or change lifecycle shutdown. If there is no healthy local server yet, optionally run `herdr-desktop-pet settings set exit_with_herdr off` **before starting the pet for the first time** to keep it alive while configuring the remote profile; once remote polling is healthy, turn it back on if desired. Use the installed native CLI (or the packaged executable below) for that setting; no Herdr plugin action or remote plugin installation is required.

The pet refreshes Herdr's saved-machine catalog every 5 seconds, even when Remote is off, and polls only selected, enabled profiles every 5 seconds when Remote is on. It remembers profile selections across app restarts and when Remote is switched off; renaming a profile keeps its selection. Disabling or removing a saved profile excludes it from observation without stopping remote sessions or processes. Local and each remote profile have separate source labels on session cards, so matching session IDs on different servers remain distinct.

**Local pending preview only — compact settings (requires `f638f0c` in the local checkout above; not `main` or public v0.1.11):** open the full panel via the preview character/bubble **Settings…** menu or the menu-bar icon when available (conditional in the original `bf23ced` preview, policy-selectable in this working copy), then use **This Mac** / **Remote machines** and select profiles under **Machines to observe**.

Each machine row separates its name, saved session, and observation state. Long names and diagnostics wrap, and the card and scroll area grow with their contents. Settings distinguish the initial catalog lookup from a confirmed empty list. A failed refresh retains the previous list with an explicit stale-list warning. Switching remote observation off shows **Observation off**, not a previous successful connection status. **How to register** explains the manual terminal setup.
Controls are reused by opaque profile ID so polling/renaming preserve focus and scroll; scrolling is clamped if the list shrinks. The layout separates right-aligned native source switches and nests profiles under **Machines to observe**, avoiding fixed notice gaps and duplicate help. It distinguishes no selection, paused, checking, observing, unavailable, and disabled states as well as catalog lookup/empty. Labels and contextual accessibility names are localized in English and Korean; registration help and folded diagnostics are read-only.

On public `main` and v0.1.11, the **Local**/**Remote** switches and profile selections determine which sources contribute to cards, counts, overall phase, and completion/outcome reactions (the local preview calls them **This Mac**/**Remote machines**). The existing card status filter only narrows which included cards are displayed; it does not change which sources are observed. Turning off every source shows **Unknown**, not a disconnected warning for a source you chose to exclude.

Remote snapshots are polled rather than streamed: transitions shorter than the 5-second interval can be missed, including brief completions. If a selected machine becomes unavailable, its last observed cards are marked **Offline**, and retained sessions count as unknown rather than live; check the error in the menu-bar panel's **Observation sources** on public `main`/v0.1.11. Once it reconnects, or when a source is reselected, the first fresh snapshot is a baseline and does not replay old completion reactions. For SSH authentication recovery, run `herdr machine reconnect '<profile-id>'` yourself in an interactive terminal; find the ID with `herdr machine list`. **Local pending preview UI only (`f638f0c`; not `main`/v0.1.11):** expand **Show error details** for read-only diagnostics; **Copy reconnect command** copies a safely shell-quoted command with native clipboard feedback and never executes it. The pet does not prompt for credentials, perform remote setup, or silently fall back to local observation for a failed remote request. Remote observation remains read-only; prompts never use machine forwarding.

Local observation still supports Herdr 0.9.0 or later. Remote observation additionally needs a local Herdr CLI supporting [saved-machine API forwarding](https://herdr.dev/docs/0.9.2/cli-reference/#saved-ssh-machines) (`herdr --machine <profile-id> api snapshot`) and a compatible running remote Herdr server; Herdr 0.9.2 documents that CLI capability. A missing or incompatible CLI is reported in settings rather than changing the local observation minimum requirement. **Lifecycle client-attach auto-start has the separate patched-host requirement above.**

## Lifecycle settings

The Settings tab has two independent switches: **Auto-start when Herdr starts** (`auto_start`) and **Quit when all Herdr servers disconnect** (`exit_with_herdr`). Both default to on for a new profile. Existing `lifecycle.json` profiles migrate legacy boolean `enabled` to `auto_start` (including `false`), and default missing `exit_with_herdr` to off to preserve their prior independent-running behavior. Both settings live only in `lifecycle.json`, not `preferences.json`; existing endpoints, startup metadata, and unrelated fields are preserved. Invalid values/JSON produce an error instead of being replaced with defaults; a failed save leaves the previous switches and running policy unchanged and shows an error.

`auto_start` on starts an absent pet at server startup or a successful shell/terminal client attach. Automatic `ensure` with auto-start off successfully skips an absent pet; it may register an endpoint for an already-running pet but never forces a hidden pet visible. Manual `start` works with auto-start off and does not change either setting. `stop` and the native **Quit** end only this run (including a pending startup); `restart` starts manually again and preserves settings. If auto-start remains on, the next qualifying server startup/client attach can start the pet again. Turn off `auto_start` to disable future automatic starts. Herdr plugin disable/unlink is a separate host action, not a pet setting; disabled plugins do not receive actions/hooks, and confirmed disable on all registered endpoints terminates an already-running pet immediately.

**Local pending preview only — watcher startup correction (`078f5c2`; not `main` source builds or public v0.1.11):**

A valid `plugin_list` response with no `desktop-pet` entry on a newly observed local endpoint means **not installed here**, not disabled: the pet can still connect, read local snapshots, and continue observing remote profiles. An explicit `desktop-pet` entry with `enabled: false` is confirmed disable and detaches that local endpoint. If this pet daemon has already seen an enabled **or disabled** desktop-pet entry at an endpoint, a later missing entry means observed unlink and detaches it as well. That history survives rechecks and reconnects during the same daemon run, but a new daemon starts with no such history. When **all registered local endpoints** are confirmed disabled/unlinked, the pet quits immediately even if a remote profile is healthy or `exit_with_herdr` is off; initial missing is not a confirmed disable/unlink. Otherwise the healthy-connection grace described below applies.

**Baseline and preview lifecycle policy:**

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

## Character packs

Use the menu-bar panel's **Characters** tab, or run the native CLI from the checkout:

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
- On public `main` and v0.1.11, use the menu-bar panel to restore visibility or turn off full-window click-through; contextual **Settings…**, **Show Character**, and the conditional recovery icon are not available.
- **Local checkout visibility recovery only (not `main`/v0.1.11):** with a standalone bubble visible, use its background/card-header **Show Character**; in the current working copy, the menu-bar icon also remains available in **Always show** mode. With both windows hidden or full-window click-through on, use the icon's **Show and Enable Character** in either mode. The original `bf23ced` preview instead had no icon in interactive states. These visibility states do not mean startup failed. On any version, if the daemon is stopped or did not start after linking, check unknown-hook warnings, invoke `start`, and inspect `status` and the daemon log. An off `auto_start` is an intentional automatic skip.
- If `start` reports `Herdr watchers are shutting down`, check which Herdr socket the CLI inherited (`HERDR_SOCKET_PATH` / `HERDR_CLIENT_SOCKET_PATH`), which CLI executable/version is on `PATH`, and which Herdr server actually owns that socket. A `herdr plugin list` against another server does not diagnose the daemon's observed endpoint. Use the intended CLI and socket/session together; for remote errors, also check the saved-machine forwarding CLI version and the selected profile. A patched Herdr executable matters for `client.attached` auto-start, but does not install desktop-pet on any server.
- For remote observation errors, check **Observation sources** and the saved profile's status; see [Observation sources](#observation-sources) for authentication recovery and CLI compatibility.
- If a release download fails, check network access to github.com. `--prebuilt` does not fall back; the default mode falls back to a source build, which requires the source toolchain.
- Use Herdr's upstream provider integrations. Restart/reload existing provider sessions after installing their upstream lifecycle hooks.

## Development

After a source installation:

Native hit-overlay blending uses typed subexpressions for compatibility with the release runner’s Xcode 16.4 Swift compiler. Keep those type boundaries when modifying the overlay; the source toolchain still requires Xcode Command Line Tools.

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

