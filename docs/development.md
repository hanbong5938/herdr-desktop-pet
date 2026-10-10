# Development

**English** · [한국어](development.ko.md) · [Installation and overview](../readme.md)

Commands below assume the repository root as the current directory. This checkout targets v0.3.4; its source version does not prove a non-draft Release, complete assets, or an updated installer/Homebrew formula. Confirm the [actual Releases](https://github.com/hanbong5938/herdr-desktop-pet/releases) before prebuilt installation. Apple Silicon macOS 13+ is the supported native platform; Intel macOS, Windows and Linux are not native distribution targets.

## Source development

The [Herdr host integration guide](../integrations/herdr/README.md#build-in-separate-checkouts) gives the actual separate-checkout procedure: build official Herdr 0.9.3 source at commit `7b116c05bfda646af39d2524c54e70c751f57ee8` with the [client-attach patch](../integrations/herdr/client-attached.patch), then set `PATCHED_HERDR="$HERDR_SOURCE/target/release/herdr"` to that built executable. Follow its locked host toolchain and isolated XDG/session instructions **before** using this variable; a link does not build the host or the pet. Stock Herdr 0.9.0 and 0.9.3 lack `client.attached`; `min_herdr_version = "0.9.3"` is the patch baseline, not evidence of hook support. Only an actual future host release advertising the hook can replace this patch. Check link warnings, the patched session's API subscription/schema, and the guide's client-attach probe; an unknown-hook warning or merely successful plugin link is not success. Use the same patched binary and session for inspection, not a different `herdr` on `PATH`.

For a linked development checkout, build this checkout first (the plugin's `[[build]]` runs on managed install, **not** on `plugin link`):

```sh
bash scripts/install.sh --source
"$PATCHED_HERDR" plugin link "$PWD" --enabled
"$PATCHED_HERDR" plugin action invoke start --plugin desktop-pet
```

The source installer installs frozen Bun and npm dependencies, builds the Rust executable and update coordinator, packages the native rig runtime and creator resources, and validates the app. It needs Rust/Cargo, Bun, Node.js/npm, Xcode Command Line Tools and `codesign`; the separate host build also has its own prerequisites in the integration guide. Linking/enabling on an already-running server does not launch the pet: invoke `start` once, or with `auto_start` on wait for a subsequent successful shell/terminal attach. Server startup also runs `ensure`. Plugin actions require an available enabled host; the packaged native binary's offline settings commands do not. Use `./dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet` for source automation. Stop an old daemon or consistently isolate **both** config and state paths for `start`, commands and `stop`; a source CLI version alone cannot establish publication or replace a running older daemon, so inspect `status` for the actual executable.

The shared [installer](../scripts/install.sh) has three modes: optionless installation tries the manifest-pinned prebuilt and falls back to a source build **only if unavailable or invalid**; `--prebuilt` fails instead; `--source` builds this checkout (Bun/npm dependencies may still be downloaded). Managed Herdr plugin installation invokes it through `[[build]]` in the plugin directory's `dist/`. The manifest targets `0.3.4`; neither fallback source compilation nor a source note proves public prebuilt availability. The installer accepts only an archive rooted at `HerdrDesktopPet.app/`, validates its checksum and native signature, and never consumes the separate rig provenance release sidecar. Client-attach auto-start still needs a patched/capable host.

The repository has the `herdr-plugin` topic for automatic discovery in the [Herdr marketplace](https://herdr.dev/plugins/); the index refreshes every 30 minutes and is an **unreviewed community listing**, not a host compatibility review. The Homebrew [personal tap](https://github.com/hanbong5938/homebrew-tap) formula installs `HerdrDesktopPet.app` inside Homebrew's prefix, **not** `/Applications`, and puts the CLI on `PATH`; it installs neither the Herdr plugin nor a patched host. Install the plugin for lifecycle hooks and patch the host for client-attach auto-start. If formula loading is refused, trust only this formula with `brew trust --formula hanbong5938/tap/herdr-desktop-pet`. For upgrades stop the daemon before upgrading; `start` reuses a running one, so inspect both CLI `--version` and the restarted `status` (`app_version` and executable). Older cask installs copied the app to `/Applications`: migrate with `brew uninstall --cask herdr-desktop-pet` followed by `brew install hanbong5938/tap/herdr-desktop-pet`; user data is retained. See the [bilingual 0.3 upgrade guide](releases/0.3.md) for stable/beta profile isolation and migration, not as evidence that source-only versions have published assets.

## Native build and signing

After source installation, the native checks are:

```sh
bun run check
bun run test:native
```

Native hit-overlay blending uses typed subexpressions for compatibility with the release runner's Xcode 16.4 Swift compiler. Preserve these type boundaries when changing the overlay; source builds still require Xcode Command Line Tools.

Debug and release native builds require `codesign`. They ad-hoc sign **only** `rig-decode-worker` with `com.apple.security.cs.allow-jit=true` from [`native/rig/RigDecodeWorker.entitlements.plist`](../native/rig/RigDecodeWorker.entitlements.plist). Cargo tracks plist changes to rebuild and re-sign the worker. JavaScriptCore can JIT-compile the trusted decoder bundle, while the main app and native rig library receive **no** JIT entitlement; worker isolation, validation, and resource limits remain. `bun run package:native` reapplies the worker-only entitlement with the selected signing identity and verifies effective entitlements and strict worker/app signatures after outer-app signing. Failure to sign or verify **stops packaging**. Default ad-hoc packaging is local use, not Developer ID signing or notarization.

The rig bridge checks version and seven C layout sizes **before** handing typed asset/token inputs across the Rust/Swift boundary; initial motion remains borrowed sealed bytes copied by the native host, never a path lookup. The Metal renderer prepasses actual eye-white apertures with independent L/R/unsided stencil bits before stable-ordered color drawing in the same encoder. The CPU semantic hit and raster reference use matching ownership; no-white rigs keep their legacy iris rendering. A `RIG_PROBE_FAULTS` probe can run `--eye-stencil-checks --output ABS` to save real-pixel and semantic fixtures; this does not establish the cause of earlier cancelled/disconnected visual observations.

`scripts/build-rig-native.mjs` records bounded source/compiler/output hashes in `native/target/rig-native/release/rig-native.json`; packaging verifies source and output closure, embeds that build record at `Contents/Resources/rig-native-build.json`, preserves the independent update coordinator and checks its protocol/signature, then writes `dist/HerdrDesktopPet.app.rig-native.json` after signing with hashes of **all** deployed `Contents` files. The sidecar stays outside the signed app to avoid a self-hash cycle. The release workflow uploads a separately named `HerdrDesktopPet-v0.3.4-rig-native.json` asset alongside the unchanged app-only archive and its two checksum files; neither updater nor installer needs a new archive layout. Compare sidecar hashes to actual unpacked, post-signing app files before asserting deployed-byte provenance.

## Repository layout

| Path | Purpose |
| --- | --- |
| [`herdr-plugin.toml`](../herdr-plugin.toml) | Herdr plugin lifecycle and actions |
| [`native/`](../native/) | Rust app, native UI, session handling and character management |
| [`native/rig/`](../native/rig/), [`web/rig/`](../web/rig/) | Rig rendering, decoding and authoring support |
| [`assets/`](../assets/) | Bundled Rubelia default and menu thumbnail only |
| [`tools/`](../tools/) | Generic character authoring and rig inspection utilities |
| [`scripts/`](../scripts/) | Installation, native builds and app packaging |
| [`.github/workflows/release.yml`](../.github/workflows/release.yml) | Release packaging workflow |

## Character authoring

Use the [character creator guide](../.agents/skills/character-creator/SKILL.md) and [`tools/character-pack.py`](../tools/character-pack.py) to author/validate packs. The packaged app includes these resources at `Contents/Resources/creator/SKILL.md` and `Contents/Resources/creator/character-pack.py`. In a source checkout the [local Qwen + See-through production skill](../.agents/skills/create-pet-character/SKILL.md) produces ten independent poses **from an approved illustration**; external engines and model weights are not bundled. Rubelia is the bundled rig template. PNG templates, Aurora, Coding Cat, optional Rubelia outfits and their artwork generators are maintained in [herdr-characters](https://github.com/hanbong5938/herdr-characters); supply an external template with `--templates-root PATH` rather than assuming those assets are bundled.

Keep each imported pack's license, attribution and source-term files with it. Bundled Rubelia's [license](../assets/rubelia-default/LICENSE.txt), [attribution](../assets/rubelia-default/ATTRIBUTION.txt), [source record](../assets/rubelia-default/source-record.json), and [background-repair record](../assets/rubelia-default/background-repair.json) remain the artwork authorities. [`LICENSE.txt`](../LICENSE.txt) retains the original Coding Cat notice; Coding Cat and its drawing source moved to the separate character repository. These notices do not give every repository component a blanket license. Artwork-owner approval does not grant model rights and software MIT does not relicense artwork. Qwen's research license remains an authoring record; no model weights or engines are bundled. Version-specific redraws, face corrections and historical limitations are documented in [v0.1.6](releases/v0.1.6.md), [v0.1.9](releases/v0.1.9.md), and [v0.1.10](releases/v0.1.10.md).

## Website (GitHub Pages)

The Korean-default/English promotional website is in [`site/`](../site/); it describes the native macOS app, not an app running in the browser. Published Pages URLs: [Korean](https://hanbong5938.github.io/herdr-desktop-pet/) · [English](https://hanbong5938.github.io/herdr-desktop-pet/en/). Headings use a compact responsive scale and tighter spacing; body text, terminal commands and control sizes remain independent of heading scale. From the repository root:

```sh
bun test tools/pages.test.ts
node tools/pages.mjs check
node tools/pages.mjs serve --port 4173
# Open http://127.0.0.1:4173/herdr-desktop-pet/ (English: /herdr-desktop-pet/en/)
node tools/pages.mjs stage /tmp/herdr-pages
```

The site CLI uses **Node.js 22**; **Bun 1.4.2** is only the HTTP regression-test runner, launching the actual Node server. CI runs this regression before staging. `check` validates local links, anchors, locale metadata and assets, **not** external links over the network. The local preview serves only regular public files; if the final `404.html` fallback is missing, a symlink or nonregular, it returns plain-text `500` with `nosniff` rather than streaming it, and `HEAD` has no body. `stage` checks first and copies only public `site/` files to a **new or empty directory outside the repository**; it does not deploy. The Pages workflow checks PRs and attaches a review artifact without deployment. Pages source is **GitHub Actions**; only eligible `main` runs deploy (merge changes to `main` or manually run the workflow on `main`). These local commands do not change Release binaries, the Wiki or repository Pages settings.
