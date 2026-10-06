# Upgrading to the local Unreleased native checkout

[한국어](unreleased.ko.md) · [Unreleased notes](../releases/unreleased.md) · [Version documentation](../releases/README.md)

**This guide applies only to local `worktree/rapid-harbor-d2a6` at `bf23ced1649fd0074aec8736644c8f02aa0c492b`, not to `main` or the public v0.1.11 binary.** The four native groups—contextual Settings/conditional rescue, independent character/bubble visibility, compact observation settings, and standalone/remote-only watcher startup—are implemented in that unmerged local checkout. Documentation and publishing tools are on `main`, but that does not merge these native features. The pinned public app and app version remain **v0.1.11**; no next version or preview download is assigned. This branch/commit is an implementation identifier, not a guarantee of public remote availability. Inline replies already shipped in v0.1.6; see the [old-composer migration](v0.1.4-to-v0.1.6.md).

For `main` or the public binary, use the [root installation and runtime guide](../../readme.md): the baseline uses its existing menu-bar settings entry point and Local/Remote observation controls. Do not expect this guide's independent visibility, conditional rescue, compact observation UI, contextual Settings, or watcher startup correction from a `main` source build. All native-preview behavior below is scoped to the identified local checkout; shared configuration, data-preservation, IME, delivery, and host-capability warnings still matter.

## Preserve the existing profile

Keep existing `HERDR_PLUGIN_CONFIG_DIR` / `HERDR_PLUGIN_STATE_DIR` or the same Herdr XDG plugin profile. Back up configuration before replacing the app. Retain `lifecycle.json`, `preferences.json`, and `characters/`; upgrading is not a reason to delete or recreate them. Saved character selection/imported revisions and personal dialogue remain in their existing stores; only saved dialogue survives restart, and reply/editor drafts are in-memory for the current run. Lifecycle switches remain owned only by `lifecycle.json`, including legacy `enabled` → `auto_start` migration retaining false and missing `exit_with_herdr` defaulting off for existing profiles (new profiles default on/on). Preserve endpoint/startup/unrelated fields; invalid settings report errors, and failed saves retain prior switches and live policy rather than resetting the profile. External `--assets` dialogue remains keyed by the activation-time canonical path; a renamed path on restart does not migrate that key.

**Only if you already possess the identified local implementation**, run `bash scripts/install.sh --source` from that checkout, using the [source installation guide](../../readme.md#install-this-checkout-from-source) for toolchain, packaging, and patched-host linking instructions. The command builds the current checkout; it does not retrieve the local branch or switch implementations, and on `main` it does **not** provide these native previews. Optionless installation normally fetches the pinned v0.1.11 binary; `--prebuilt` never falls back to source. Keep the existing native executable, action IDs, and character pack format. No app/host release, remote plugin auto-install, new configuration profile, or SSH transport change is required by this guide. Do not assume downgrade compatibility for data written by a newer checkout.

## Visibility is independent, not lifecycle

| Character | Bubble | Local pending-checkout result |
| --- | --- | --- |
| Visible | Visible | Attached bubble |
| Visible | Hidden | Character only |
| Hidden | Visible | Movable, tailless standalone bubble with session cards/replies |
| Hidden | Hidden | Neither window shown; app/session still running, rescue icon available |

`show` / `hide` / `toggle` affect only character visibility. Bubble visibility remains controlled separately by `show_bubble` / `hide_bubble` or **Bubble → Bubble visible**. Hiding the character does not hide an enabled bubble. Its first detached origin derives from the attached bubble layout, not the character origin; that standalone origin is saved separately and reused on later detachments. Showing the character reattaches it. Dragging the standalone background moves only the bubble. Compact/expanded changes and reply reflow preserve the origin unless screen clamping is needed; display removal/layout changes recover an offscreen bubble onto a remaining screen.

Card selection, compact/expanded/reply state, drafts, focus, and active IME survive character hide/show during the run. Drafts do not survive restart. Placement settings changed while standalone wait for reattachment. `reset` resets the character position and clears/reseeds the standalone origin from attached layout without changing either visibility preference; it does not copy the character origin into the standalone preference. **Close Bubble Window** still hides only the bubble, preserving the app/character/session and run-local drafts, and persists bubble visibility.

Full-window passthrough applies to both windows, including standalone bubble and inline replies. Alpha passthrough applies only to character artwork, not the bubble; its pointer polling can race with click routing, so leave it off if that is unacceptable.

## Settings and conditional rescue

Right-click or Control-click the character, bubble background, or card header and choose **Settings…** to open the same full three-tab panel. Text fields retain native editing menus; buttons, dropdowns, and scrollbars retain native interactions. Canceling the bubble menu preserves selection/drafts. Bubble **Settings…** and **Close Bubble Window** are disabled while IME marked text is active and recheck that condition when chosen. Finish composition before reopening the menu. App activation is requested with the current macOS API and a macOS 13-compatible path; macOS decides whether keyboard focus transfers.

The rescue menu-bar icon appears **only if both windows are hidden or full-window passthrough is on**, including after restart and while Settings is open. Both-visible, character-only, and interactive standalone-bubble-only states use no menu-bar slot. Alpha pointer polling alone never toggles the icon. A standalone bubble offers **Show Character** in its own menu.

**Show and Enable Character / 캐릭터 표시·조작 복구** shows the character and turns off full-window passthrough. It preserves bubble visibility and alpha passthrough: a visible bubble reattaches, a hidden bubble stays hidden. This is a partial preference change, not a full reset. The rescue menu also offers **Settings…** and **Quit**. CLI `show` is not the rescue action: it changes character visibility without enabling the bubble or clearing full-window passthrough.

## Remote observation keeps saved choices and stays read-only

**This Mac** defaults on; **Remote machines** defaults off. Their right-aligned native switches are separate from saved profiles nested under **Machines to observe**. Rows distinguish machine name, saved session, and status. Content-sized rows/cards/scrolling replace fixed gaps and duplicate help; controls keyed by opaque profile ID retain focus/scroll during polling or renaming, and scrolling clamps if the list shrinks. Labels/contextual accessibility names are Korean/English.

The UI distinguishes initial catalog lookup, confirmed empty, no selection, paused, checking, observing, unavailable, and disabled. A failed catalog refresh retains the previous list with an explicit stale-list warning. Remote off preserves selections but masks stale successful statuses with observation-off feedback. Registering/renaming a machine does not implicitly select a new profile. Disabling/removing profiles excludes them without stopping remote processes. All sources off yields Unknown; source switches affect display counts/state/reactions, while the card filter only narrows displayed cards.

Set up SSH, add/authenticate machines, and approve any server setup manually in an [interactive Herdr terminal](https://herdr.dev/docs/0.9.2/connecting-machines/). Each selected enabled saved profile observes one saved remote session, not its whole host. The remote needs a compatible running server, not the pet app/plugin. Catalog refresh and selected-profile polling run about every five seconds (catalog refresh continues with Remote off); brief transitions can be missed. Retained disconnected cards are last-known offline/unknown data, not live status. After recovery/reselection, the first snapshot is a baseline, not a replay of old completion reactions.

Registration help and folded diagnostics are read-only. **Copy reconnect command** copies a shell-quoted `herdr machine reconnect '<profile-id>'` command with native clipboard feedback; it **never executes it**. Find IDs with `herdr machine list`, then run recovery yourself in a terminal. The pet does not prompt for credentials, install remote software, change SSH transport, or fall back to Local on remote failure. Remote cards remain observation-only; prompts never use machine forwarding.

## Remote-only startup and watcher shutdown

Run the pet on the **local Mac**. Authenticate the saved machine in a terminal, enable **Remote machines**, then explicitly select its enabled profile. Attaching to a remote session or registering a machine alone does not enable its observation. If there is no healthy local server yet, optionally use the native CLI **before the first start**:

```sh
herdr-desktop-pet settings set exit_with_herdr off
```

A source install can use `./dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet` instead of the installed CLI. The native settings command works with the pet stopped, no host, or a disabled plugin and does not launch a pet; a Herdr plugin action still requires a live enabled host. Once remote polling is healthy, optionally set `exit_with_herdr on` again.

**This Mac off excludes local display data, not local endpoint lifecycle.** A valid local `plugin_list` with no desktop-pet entry on first observation is not disable: keep local snapshots/subscription and remote polling available. Explicit `enabled: false` detaches that endpoint. A missing entry after this daemon previously observed either an enabled or disabled row is observed unlink and detaches too. Membership history survives rechecks/reconnects within this daemon, not a new daemon.

If **every registered local endpoint is confirmed disabled/unlinked**, the pet quits immediately, even with healthy remote polling or `exit_with_herdr` off. Initial missing is not that terminal condition. Other loss-of-health cases use the existing 30-second grace when exit is on, including initial no-server start; healthy reconnection cancels it and one healthy server prevents exit. Client detach is not server loss; stale endpoints/failed retries never extend grace. Switching exit off cancels a countdown, and enabling it while disconnected starts a fresh 30 seconds. Stop/Quit end only this run; auto-start can relaunch on a later qualifying event unless turned off. Changing auto-start never forces the current pet to stop or show.

## Diagnose capabilities separately

Local observation supports Herdr 0.9.0+. Remote polling needs a local CLI with [saved-machine API forwarding](https://herdr.dev/docs/0.9.2/cli-reference/#saved-ssh-machines) (`herdr --machine <profile-id> api snapshot`, documented in 0.9.2) and a compatible remote server. Local replies separately need `agent.prompt`; the original evidence verified it with 0.9.2, not every server meeting the observation baseline. Offline/stale/unready/unsupported targets cannot send; no shell/SSH/raw-pane fallback or automatic retry exists. Check unknown delivery before resending, and handle approvals/questions in the real terminal. The API has no atomic expected-session guard, so last-instant pane occupant replacement remains a limit.

Client-attach auto-start separately requires official Herdr 0.9.3 source with the [client-attached patch](../../integrations/herdr/client-attached.patch) applied and built, or an actual host release advertising that hook. Stock 0.9.0/0.9.3 do not implement it; a manifest minimum version is not capability proof. Follow the [patched-host guide](../../integrations/herdr/README.md), use its `PATCHED_HERDR` rather than stable `herdr` on PATH, inspect link warnings, and verify subscription. Linking/enabling an already-running host alone does not launch the pet: start once or await a later qualifying attach.

For `Herdr watchers are shutting down`, identify the inherited `HERDR_SOCKET_PATH` / `HERDR_CLIENT_SOCKET_PATH`, the CLI executable/version on PATH, and the server owning that socket separately. A plugin list from another server cannot diagnose the daemon's observed endpoint. Check the intended CLI/socket/session together and the selected remote profile/forwarding version; a patched host does not install desktop-pet on any server.
