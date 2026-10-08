# Herdr Desktop Pet

A native macOS desktop companion for [Herdr](https://herdr.dev). Rubelia lives on your desktop, reacts to touch, displays observed Herdr session activity, and lets you submit text to a selected agent session from its status bubble.

**English** · [한국어](readme.ko.md) · [Version documentation](docs/releases/README.md) · [Releases](https://github.com/hanbong5938/herdr-desktop-pet/releases)

> **Stable v0.2.0 is the current release and default/prebuilt installation target; this checkout also carries the unreleased additions below.** Its released native feature groups include independent character/bubble visibility, compact observation settings, standalone/remote-only watcher startup, contextual Settings, configurable menu-bar recovery and image, linked-worktree removal, reply layout and clipboard shortcuts, and malformed-link scanner fixes. The earlier v0.1.11 binary and immutable beta1/beta2/beta3 releases remain historical, separate channels. Consult the [v0.2.0 release notes](docs/releases/v0.2.0.md), [upgrade guide](docs/migrations/v0.1.11-to-v0.2.0.md), and [historical local-preview guide](docs/migrations/unreleased.md). Release-catalog assets, rather than this source guide, establish publication availability.

> **Unreleased checkout only:** This working copy adds session-list Search/Sort/Running first, native automation commands, conflict-aware dialogue drafts, drag-resizable status bubbles, and a native character browser with an explicit verified official **Download & Apply** action. They are **not** part of the pinned published v0.2.0 or historical beta1/2/3 binaries, even though this checkout still reports version `0.2.0`. The four older native preview groups, menu-bar/worktree additions, and beta3 reply fixes **are already in stable v0.2.0**. Build this checkout explicitly with `bash scripts/install.sh --source`; the default installer and `--prebuilt` use the pinned release. Stop an older daemon before running the source executable (or use matching isolated config **and** state directories), and inspect `status` for the actual running executable rather than trusting `--version`. See [unreleased changes](docs/releases/unreleased.md#current-unreleased-checkout--현재-미출시-체크아웃) and the [native CLI contract](docs/cli.md#english). No new beta publication or physical GUI/IME acceptance is established by this checkout documentation.

<img src="assets/rubelia-thumbnail.png" alt="Rubelia, the default desktop companion" width="220">

## Features

- Native desktop character with a menu-bar control panel.
- Observe local Herdr sessions by default, with optional saved-machine remote observation.
- Session-aware states: idle, running, waiting, and unknown.
- Readable session cards with task titles, workspace/tab context, source labels, and a separate status label.
- Inline replies beneath selected local agent session cards in the expanded status bubble, shipped since v0.1.6; remote cards remain read-only. See the [v0.1.4 → v0.1.6 migration](docs/migrations/v0.1.4-to-v0.1.6.md).
- Head/body tap reactions, head petting, dragging, and resizing.
- Configurable status bubble placement and visibility, plus independent drag resizing in this unreleased checkout.
- Optional status icons and colors, with a status summary that remains visible alongside dialogue.
- Full-window and alpha-mask click-through modes.
- English and Korean UI options.
- Importable PNG and rig character packs with revision history; this unreleased source checkout also offers a native browser for installed and official characters.
- Independent character and bubble visibility, with a standalone bubble and selectable always-visible/recovery-only menu-bar icon and custom PNG artwork.
- Compact local/remote observation settings, copy-only reconnect guidance, and standalone remote-only watcher support.
- Contextual Settings and guarded linked-worktree removal for eligible local worktrees; removal deletes ignored files too.
- Edit per-character Korean and English dialogue without modifying character packs.

Rubelia is the only bundled character and the default model (`default@0`). Optional characters and wardrobe packs are maintained in the separate [character repository](https://github.com/hanbong5938/herdr-characters). The published stable app still requires manual import; only this unreleased source checkout offers an in-app official catalog and explicit verified download/apply. The bubble reports observed Herdr session status; it does not infer provider results. A successful message submission means Herdr acknowledged the prompt, not that the agent finished processing it.

Session cards use the observed terminal title, then a named tab, then workspace/directory context. Missing names receive an explicit fallback; duplicate names gain a visible discriminator. Hover a card for the full title, working directory, and internal IDs. Renaming a title preserves selection and scroll position; disconnected sessions retain their last observed title with an offline status.

Opening the menu-bar control panel requests app activation using the current macOS API, with a compatible activation path on macOS 13. macOS decides whether to grant activation; opening the panel does not guarantee a keyboard-focus transfer from another app.

### Session list search and ordering (unreleased source checkout only)

This newer native control in the expanded bubble is **not in stable v0.2.0 or the immutable beta1, beta2, and beta3 binaries**. Build this checkout explicitly with `--source`; the public default/prebuilt installer still selects stable v0.2.0. The historical inline replies below remain available in their shipped versions. No `beta/0.3.0-beta.1` asset or hands-on GUI/IME acceptance is claimed here.

Use **Search** to narrow cards by a case-insensitive substring of their title, tab or workspace name, agent name, source display name, full working directory, or fallback name (surrounding search whitespace is trimmed). IDs and socket paths are not searchable. The **Status** filter and search intersect; neither changes the observed overall status summary or reactions. The **Sort** menu contains **Stable** (original order), **Name**, and **Source**, followed by the independent **Running first** option below a separator; there is no separate checkbox. When enabled, running cards from all observed sources come ahead of other cards *before* the selected sort orders each group. The closed menu title and accessibility value show both choices, for example **Name · Running first**. All six combinations and existing saved settings remain supported; this is running priority, not a full ordering of every status. The GUI displays at most 128 cards after filtering and sorting; the matched/omitted indicator counts all matches and those beyond the visible limit. Unlike that GUI cap, the unreleased `sessions list --limit` CLI option sets page size and collects all coherent pages.

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
| App | Pinned stable v0.2.0 prebuilt (plugin installer or Homebrew; no build toolchain), or build this checkout with Rust/Cargo, Bun, Node.js/npm, and Xcode Command Line Tools |

**The patch is REQUIRED for client-attach auto-start.** Stock Herdr 0.9.0 and 0.9.3 do not provide `client.attached`; a manifest version floor is not proof of hook support. Apply the supplied patch to official 0.9.3 source and build/run that host following its build instructions; inspect plugin link warnings and verify the `client.attached` subscription is accepted. Do not treat an unknown-hook warning as success. Intel Macs, Windows, and Linux are not supported by the native distribution.

## Install as a Herdr plugin (recommended)

Install from the public [plugin repository](https://github.com/hanbong5938/herdr-desktop-pet) like any other Herdr plugin:

```sh
herdr plugin install hanbong5938/herdr-desktop-pet
herdr plugin action invoke start --plugin desktop-pet
```

Herdr clones the repository into its plugin directory and runs the manifest's `[[build]]` step, `bash scripts/install.sh`. The installer downloads the release pinned to the manifest `version` (v0.2.0) anonymously over HTTPS with `curl`, verifies its SHA-256, archive paths and entry types, arm64 architecture, code signature, and default character, and installs it into the plugin directory's `dist/`. No GitHub login or build toolchain is needed in the normal case, and nothing is installed outside the plugin directory. If the pinned prebuilt is unavailable or fails verification, the installer prints a notice and falls back to a source build, which requires the source toolchain listed above. The app is **ad-hoc signed, not notarized**; because it is fetched by `curl`, not a browser or cask download, it has no quarantine attribute and macOS shows no Gatekeeper prompt. For client-attach auto-start, run these commands against the patched Herdr host described above.

The repository is tagged `herdr-plugin` for automatic discovery in the [Herdr marketplace](https://herdr.dev/plugins/). The index refreshes every 30 minutes; this is an unreviewed community listing.

## Install with Homebrew

Install the stable v0.2.0 prebuilt app and `herdr-desktop-pet` CLI from the [personal tap](https://github.com/hanbong5938/homebrew-tap) formula; no GitHub login or source-build toolchain is required. Check the [release catalog](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/v0.2.0) for published assets before relying on a prebuilt:

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

Confirm `--version` is `0.2.0` and `status` reports `app_version` and the running executable as stable v0.2.0, not the previous binary. If Homebrew refuses to load the formula, scope trust to this formula: `brew trust --formula hanbong5938/tap/herdr-desktop-pet`. The formula installs `HerdrDesktopPet.app` inside Homebrew's prefix, not `/Applications`, and puts the CLI on `PATH`. It installs only the app, not the Herdr plugin or patched host; use plugin installation above for lifecycle hooks, and the host patch for client-attach auto-start. The app is ad-hoc signed, not notarized; formula installation does not quarantine it. `brew uninstall herdr-desktop-pet` keeps character packs, preferences, and lifecycle state. See the [upgrade and rollback guide](docs/migrations/v0.1.11-to-v0.2.0.md).

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

For development checkouts, `plugin link` does not run the manifest's `[[build]]`, so build the checkout yourself. Current public `main` includes the native stable v0.2.0 feature groups described below; `--source` builds your actual checkout, not a different release. For client-attach auto-start, run the patched Herdr host in an isolated profile (follow the [patched-host deployment guide](integrations/herdr/README.md) and use that binary explicitly, rather than an unpatched `herdr` on `PATH`):

```sh
bash scripts/install.sh --source
"$PATCHED_HERDR" plugin link "$PWD" --enabled
"$PATCHED_HERDR" plugin action invoke start --plugin desktop-pet
```

The installer installs pinned JavaScript dependencies, builds the Rust executable, packages the native rig runtime and creator resources, and validates the app. Linking/enabling on an already-running server does not itself launch the pet: invoke `start` once, or wait for a subsequent successful shell/terminal client attach when `auto_start` is on. Server startup also runs automatic `ensure`. Inspect link warnings: an unknown `client.attached` hook means the host lacks the required patch. Herdr plugin actions require a running, enabled host; direct native settings commands below work without one.

For unreleased automation commands use the newly built checkout executable at `./dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet`, not a prebuilt or older daemon. Stop the existing pet first, or keep both config and state paths isolated and consistent for `start`, subsequent commands, and `stop`. Because both binaries may report `0.2.0`, run that source executable's `status` after starting the intended daemon and check its reported running executable.

### Historical local-preview provenance

The standalone/remote-only watcher, independent visibility, compact observation settings, and contextual Settings/conditional recovery originated in four commits (`078f5c2`, `e828c77`, `f638f0c`, `bf23ced`) of the old local `worktree/rapid-harbor-d2a6` checkout. Later menu-bar mode/image and worktree-removal additions were not in `bf23ced`. Those features shipped in beta3 and are now part of stable v0.2.0 `main`; no special local branch or local-only fetch instructions are needed. A historical checkout at `bf23ced` alone still has conditional recovery and lacks later additions. See [historical preview provenance](docs/migrations/unreleased.md).

### Shared installer behavior

`bash scripts/install.sh --prebuilt` downloads the version pinned by this checkout's `herdr-plugin.toml` anonymously over HTTPS with `curl` (no GitHub login) from the checkout's git `origin` repository (falling back to `HERDR_PET_REPOSITORY`, then `hanbong5938/herdr-desktop-pet`). It validates SHA-256, archive paths and entry types, arm64 architecture, code signature, and default character, and **never** falls back to a source build. Running the prebuilt app needs no source-build toolchain; client-attach auto-start still requires the patched host. Optionless installation uses the pinned prebuilt and falls back to source only if it is unavailable or invalid; explicit `--source` builds local changes. App bundles are **ad-hoc signed, not Developer ID signed or notarized**. Installer `curl`, git, and Homebrew formula downloads set no quarantine attribute; a browser-downloaded archive may still require explicit approval to open the app.

Version-specific changes and historical artwork limitations belong in the [release notes](docs/releases/README.md), not this live usage guide. For the old multiline composer’s Enter/Escape changes, see [v0.1.4 → v0.1.6](docs/migrations/v0.1.4-to-v0.1.6.md).

## Controls

These controls describe this checkout's native app; the menu-bar click behavior and bubble resizing below are checkout changes, not claims about the published stable v0.2.0 binary. Inline replies first shipped in v0.1.6. **Close Bubble Window** hides only the bubble, keeps selection/drafts for this run, and is disabled/rechecked during IME composition. Character and bubble visibility are independent; hiding the character can leave a standalone bubble.

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
| Bubble bottom-right grip (unreleased checkout) | Resize the bubble width and height independently, including with Option held |
| Expanded bubble session card (v0.1.6+) | Open a one-line reply beneath the selected local agent session; switching cards waits for active IME composition to end, while remote cards show read-only feedback |
| Inline reply (v0.1.6+) | Enter, Command+Enter, or Send submits to the selected local session; IME composition does not submit |
| Escape in the reply / click outside the bubble (v0.1.6+) | Escape cancels active IME composition first; otherwise either folds only the reply field. Inside-bubble clicks do not automatically fold it; outside clicks do not fold during composition |
| Right-click / Control-click the bubble background or a card header | **Close Bubble Window** hides only the bubble; **Restore automatic size** clears both compact and expanded size overrides |
| Full-window click-through | Pass clicks through both the pet and bubble windows, disabling their interaction |
| Alpha click-through | Pass clicks through transparent character artwork regions only; the bubble remains interactive |

Compact and expanded bubble sizes are saved separately and shared between attached and standalone bubbles. The grip does not change character scale or font size. Expanded message and card scroll viewports use the available space instead of the automatic layout's height caps. A smaller screen limits the displayed size without discarding the requested size; it can recover when enough screen space is available.

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

In this **unreleased checkout**, external dialogue saves no longer silently replace a local editor draft. A revision/entry conflict blocks stale Save; **Reload saved** explicitly takes the latest committed text and **Rebase draft** keeps the local text against the new baseline for revalidation before Save. The editor preserves text, selection/focus and undo history while exposing a conflict; active IME marked text defers reload/rebase until composition ends. These conflict controls are not in the pinned published v0.2.0 app. See the [CLI dialogue contract](docs/cli.md#english).

Custom text replaces only the corresponding dialogue entries. It does not change observed session counts, state transitions, reaction timing, or disconnected-host notices.

## Observation sources

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


### Native automation CLI (unreleased source checkout only)

Use the executable from `bash scripts/install.sh --source`, not the pinned prebuilt; `herdr plugin action invoke` remains the separate Herdr lifecycle interface. The checkout's native `herdr-desktop-pet --help` lists the actual command options. `presentation get/set/reset/status` controls absolute visibility, placement and scale; `preferences get/set/status` controls saved/effective settings; `sessions list/show/prompt/status` reads observed local sessions and submits only to a specifically identified local agent; `dialogue list/get/set/reset-entry/reset-character/status` edits per-target overrides; `worktree inspect/remove/status` requires an expiring one-use inspect token before irreversible removal. There is no public `orchestrate` command: automation uses the private same-user Unix control endpoint, not HTTP, a shell fallback or a new service.

| Command shape | Important boundary |
| --- | --- |
| `presentation set --visible on --expected-revision N --operation-id OPID --wait 15` | At least one absolute field is required; `presentation status OPID --instance ID` checks this daemon's operation. `--scale` is an absolute finite value. Accepted explicit fields persist from the final normalized scene; a failed save does not leave a ghost field in a later request. |
| `preferences get`; `preferences set --menu-bar always --expected-revision N --operation-id OPID` | Menu-bar mode and other preference fields use this **checkout-only** command; `preferences status --instance ID --operation-id OPID` is family-scoped. CLI `--language` accepts exactly `system|ko|en` before storage access; repeated `--machine ID` replaces the **whole** list and rejects any unknown/disabled ID. The GUI can retain/remove previously saved unavailable IDs while adding only currently enabled IDs. |
| `sessions list --instance ID --limit 128`; `sessions show --instance ID --source N --generation N --terminal TERMINAL_ID` | 1–128 is a **page size**, not a total row cap. Each response page fits a 512 KiB encoded frame including JSON escapes, cursor and newline; a smaller prefix is returned when necessary, and the CLI collects all coherent pages. `sessions prompt` requires the same four identity flags and exactly one of `--text`, `--file`, `--stdin`; an ACK is not agent completion. |
| `dialogue list`; `dialogue get --target 'IDENTITY_JSON' --locale ko --slot idle` | `--target` is the full serialized **identity** from `dialogue list`, not merely a pack ID. A Character(ID) override is shared across authored revisions of that ID, while the authored reference/generation and optional `--baseline` CAS remain exact; omitted baseline reads fresh once, with no mutation retry. Native application is evidenced by the active renderer token and that target's override token, not merely a successful save. |
| `worktree inspect --instance ID --source N --generation N --terminal TERMINAL_ID` | Review the returned frozen checkout/workspace and token before separately running `worktree remove --token TOKEN`; no path, force, trash or undo. Ignored files can be deleted even with backend `force: false`. Never use a live user's checkout as a trial target. |

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

For these pack commands, `CHARACTER_ID` is positional (`pack select ID`, `pack restore ID --revision N`); there is no `--id` for selection. In the **unreleased checkout**, pack mutations also accept `--operation-id ID`, `--expected-generation N`, and `--wait SECONDS` or `--no-wait`. Default offline mutation completes its own worker synchronously; explicit async/finite waits need a running daemon and never secretly start one. After ACK, a finite wait uses one absolute deadline through sleeps, complete status replies and decoding, without resending; a late terminal reply cannot turn an elapsed wait into success. `--no-wait` does not mask a terminal failure already present in the initial reply: failed/canceled/uncertain terminal states exit nonzero, whereas accepted/pending is only an ACK. Use `pack status OPID` for uncertain outcomes; acceptance is not native application or persistence. These additional controls are not promised by pinned v0.2.0 binaries.

The browser's **Official Catalog** is a fixed source from [herdr-characters](https://github.com/hanbong5938/herdr-characters), not an arbitrary URL or deep link. Currently its `packs-v0.0.2` release offers Coding Cat; Rubelia remains the sole bundled/default character. The catalog and static portrait preview may load during browsing, but no pack archive is fetched until you explicitly click **Download & Apply**. This online-only action pins the catalog entry and checks the release download's actual size, SHA-256 and pack manifest identity before native preparation and one generation-checked install-and-select commit. A completed operation means the chosen revision actually became active, not merely that a download or submission was acknowledged. Progress and **Cancel Download** are available before commit; closing the browser hides it but does not cancel ongoing work, and an already committed change cannot be canceled. If the operation has an unknown outcome or the committed selection is pending native apply, inspect its operation status instead of blindly retrying. If an official ID already exists locally, that does **not** prove official version or provenance: the card offers **View local pack** to navigate to the actual installed card, never silently overwrites or updates it. Unsupported format/render-mode combinations cannot be installed. An offline or unavailable catalog does not block local browsing, import, selection, or management. The pinned published stable app has no character browser or official download action: manually import optional packs there. No new CLI command, arbitrary URL source, or automatic update is provided.

- Manual Import accepts a character directory or `.herdrchar` archive; importing does **not** select or activate it.
- In the **unreleased browser**, installed cards and historical revisions only stage a candidate until **Apply**. Native CLI `pack select` immediately selects the newest revision, and `pack restore` immediately selects an existing historical revision; these commands do not require UI Apply.
- Removing the active pack switches back to Rubelia.
- The built-in `default@0` cannot be imported, updated, or removed.
- Keep operation IDs. If a mutation's outcome is uncertain, run `"$PET" pack status OPERATION_ID` instead of blindly repeating it.

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

