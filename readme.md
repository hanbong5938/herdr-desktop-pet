# Desktop Pet — Herdr integration

[English](readme.md) · [한국어](readme.ko.md) · [日本語](readme.ja.md) · [简体中文](readme.zh-Hans.md)

**Publication boundary:** legacy [Herdr v0.3.6](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/v0.3.6) is shipped with protocol 3 / observation capability 1. Independent Desktop Pet v0.4.0 (protocol 4 / observation capability 2 / integration API 1) and the legacy bridge v0.3.7 are reviewed candidates, not verified public installations. Commands below describe the neutral cutover; do not treat notes, a source version or a tag as publication proof.

Install only the integrations you want. Herdr is a thin consumer; Tern does not download, build, install or auto-start the native app. Plugin updates/removal affect integrations only, never the shared app, profile or characters. Native updates do not reinstall Herdr plugins.

[Desktop Pet / core app](https://github.com/hanbong5938/desktop-pet/blob/main/readme.md) · [Herdr host / patch / OMP / link](integrations/herdr/README.md) · [Legacy A → B migration](https://github.com/hanbong5938/desktop-pet/blob/main/docs/migrations/desktop-pet.md)

[Core app site](https://hanbong5938.github.io/desktop-pet/en/) · [Core Wiki](https://github.com/hanbong5938/desktop-pet/wiki) · [Herdr integration landing site](https://hanbong5938.github.io/herdr-desktop-pet/en/). The original site contains only integration/migration entrypoints, not duplicate app UI or artwork.

After the neutral release and formula are actually published, the public command is:

```sh
brew install hanbong5938/tap/desktop-pet
herdr plugin install hanbong5938/herdr-desktop-pet
herdr plugin action invoke start --plugin desktop-pet
```

Apple Silicon Mac, macOS 13+. Source builds require Rust/Cargo, Bun, Node.js/npm and Xcode Command Line Tools; prebuilt installation does not. The app is ad-hoc signed, not Developer ID signed or notarized. Herdr snapshots need a compatible local server (0.9.0+); replies separately need `agent.prompt`. Optional client-attach auto-start still needs [the integration-owned patch and host instructions](https://github.com/hanbong5938/herdr-desktop-pet/blob/main/integrations/herdr/README.md) on official Herdr 0.9.3 or an actual hook-supporting host. Stock 0.9.0/0.9.3 lack that hook; plugin installation does not patch the host.

For prebuilt or source installation, use the **core** checkout, not a Herdr integration checkout. Default installation is prebuilt-only; download or validation failure never silently compiles source. `--source` explicitly builds only the current core checkout and uses its `dist/DesktopPet.app`.

## Herdr consumer contract

This repository does not build or copy `DesktopPet.app`, native packages or character assets. Its integration-only manifest, wrappers and adapter now live in this checkout; native source/artwork/tooling belong to core. The consumer pin still requires actual public core installer/checksum and reviewed release API metadata before materialization/public installation. Destination links and local source are not publication proof.

The consumer discovers the exact compatible shared native installation in this order: explicit absolute executable → validated account-home shared locator → native `desktop-pet` on PATH → actual Brew prefix → account-home `Applications/DesktopPet.app`. Invalid explicit selection or locator fails closed. Only an absent installation permits bootstrap from the pinned, digest-verified **core** installer; incompatible installs require explicit native upgrade/repair, never a product build or silent fallback here.

Herdr profile precedence is injected `HERDR_PLUGIN_CONFIG_DIR` / `HERDR_PLUGIN_STATE_DIR` → explicit `--config-dir` / `--state-dir` → Herdr XDG/HOME defaults. The wrapper passes both resolved paths explicitly to the core; it never moves or merges profiles. Backend provenance uses `--herdr-socket ABS`, not legacy `--socket` or the pet's private control socket. `status` identifies the actual running image/profile, not merely the selected CLI.

The [Herdr guide](integrations/herdr/README.md) retains all 21 action IDs and their visibility/bubble/lifecycle semantics, host build/patch, link and OMP instructions. `ensure` is idempotent and does not reveal a hidden pet. Optional attach auto-start requires a real hook-capable host. Fresh absent lifecycle files default Herdr exit policy to true; an existing file missing that key defaults false. Explicit policy and unknown fields survive writes. Tern is a separate consumer and never auto-starts the shared app.

## Source / link

Install or build the core first (`bash scripts/install.sh --source` in the **desktop-pet core** checkout). Then link **this integration checkout**, not the core, using the selected compatible app. `plugin link` does not run a build. Public/bootstrap use remains gated on actual consumer pin materialization. See [separate checkouts](integrations/herdr/README.md#build-in-separate-checkouts); plugin installation does not patch Herdr.

Common [usage](https://github.com/hanbong5938/desktop-pet/blob/main/docs/usage.md), [CLI](https://github.com/hanbong5938/desktop-pet/blob/main/docs/cli.md), [development](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.md) and native feature/provider details belong to the independent core, not this source tree.

## Ownership

This repository owns `native-adapter`, `herdr-plugin.toml`, thin install/run wrappers, `integrations/herdr` and `integrations/omp`, integration validation, and frozen historical releases. App/UI/rig/artwork/packaging/source instructions belong to [desktop-pet](https://github.com/hanbong5938/desktop-pet). `.herdrchar`, pack v5, `default@0`, rig ABI and [herdr-characters](https://github.com/hanbong5938/herdr-characters) catalog/gallery authority are unchanged. Historical [v0.3.6](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/v0.3.6) and older release bodies/tags/assets remain immutable; an integration-only v0.4.0 does not carry a native archive.

## Artwork and licensing

[Software license](LICENSE.txt) and artwork/model terms are **distinct**: MIT software terms do not relicense character art. Bundled Rubelia's [license](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/LICENSE.txt), [attribution](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/ATTRIBUTION.txt), [source record](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/source-record.json), and [background-repair record](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/background-repair.json) remain authoritative. `LICENSE.txt` also retains the original Coding Cat notice; its artwork/source moved to the [character repository](https://github.com/hanbong5938/herdr-characters). Keep each optional pack's own terms with it. Artwork-owner approval does not grant model rights; Qwen's research license is an authoring record, and model engines/weights are not bundled. See [character authoring and licenses](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.md#character-authoring).
