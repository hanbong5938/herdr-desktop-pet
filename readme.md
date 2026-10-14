# Desktop Pet — Herdr integration

[English](readme.md) · [한국어](readme.ko.md) · [日本語](readme.ja.md) · [简体中文](readme.zh-Hans.md)

**Publication boundary:** [Core v0.4.2](https://github.com/hanbong5938/desktop-pet/releases/tag/v0.4.2) is public: source `cb10ce2c3cc65402cbb914d60301f7627d5546d5`, release `412241744`, protocol 4 / observation capability 2 / integration API 1. Current metadata/evidence are anonymously materialized, reviewed v0.4.2 authority; v0.4.1 standalone/Manager evidence remains history only. Published Herdr v0.4.0 retained a generated native `dist` source fault; its tag/body/assets remain unchanged. The [corrected Herdr v0.4.1 candidate](docs/releases/v0.4.1.md) passed source/TypeScript/105 adapter gates but is still untagged/unpublished; no new Herdr ID/final SHA is known. Bridge v0.3.7 is also unpublished and must use exact published Core plus the actual corrected Herdr ref once available. Tern v0.2.0 is unchanged with its historical compatible Core v0.4.1 pin. Real owned Brew v0.4.1 → v0.4.2 reached same/default-profile Auto UI/control readiness at PID `6863` with zero hosts; standalone native-update and migration/two-host completion remain unproved.

Install only the integrations you want. Herdr is a thin consumer; Tern does not download, build, install or auto-start the native app. Plugin updates/removal affect integrations only, never the shared app, profile or characters. Native updates do not reinstall Herdr plugins.

[Desktop Pet / core app](https://github.com/hanbong5938/desktop-pet/blob/main/readme.md) · [Herdr host / patch / OMP / link](integrations/herdr/README.md) · [Legacy A → B migration](https://github.com/hanbong5938/desktop-pet/blob/main/docs/migrations/desktop-pet.md)

[Core app site](https://hanbong5938.github.io/desktop-pet/en/) · [Core Wiki](https://github.com/hanbong5938/desktop-pet/wiki) · [Herdr integration landing site](https://hanbong5938.github.io/herdr-desktop-pet/en/). The original site contains only integration/migration entrypoints, not duplicate app UI or artwork.

Core Pages actually deployed `cb10ce2`; this Herdr thin-site candidate is not yet deployed (the public source remains historical `f65f123`). Core's Release job succeeded, but its later Wiki job failed for missing dedicated `WIKI_PUBLISH_TOKEN`. Complete catalog rendering is not Wiki publication; the Wiki link is not a claim of initialized/published Core Wiki.

The neutral formula now targets the actual published Core v0.4.2 ([tap commit](https://github.com/hanbong5938/homebrew-tap/commit/32919c745864b54a2e716fcc5a3714d0d38d3e84)). The parent observed real Brew upgrade, signature/receipt/derivation validation, `brew test` and published-wrapper native readiness, scoped to zero hosts above. The optional Herdr commands install the repository's public integration, **not this still-unpublished corrected v0.4.1 candidate**:

```sh
brew install hanbong5938/tap/desktop-pet
herdr plugin install hanbong5938/herdr-desktop-pet
herdr plugin action invoke start --plugin desktop-pet
```

Apple Silicon Mac, macOS 13+. Source builds require Rust/Cargo, Bun, Node.js/npm and Xcode Command Line Tools; prebuilt installation does not. The app is ad-hoc signed, not Developer ID signed or notarized. Herdr snapshots need a compatible local server (0.9.0+); replies separately need `agent.prompt`. Optional client-attach auto-start still needs [the integration-owned patch and host instructions](https://github.com/hanbong5938/herdr-desktop-pet/blob/main/integrations/herdr/README.md) on official Herdr 0.9.3 or an actual hook-supporting host. Stock 0.9.0/0.9.3 lack that hook; plugin installation does not patch the host.

For prebuilt or source installation, use the **core** checkout, not a Herdr integration checkout. Default installation is prebuilt-only; download or validation failure never silently compiles source. `--source` explicitly builds only the current core checkout and uses its `dist/DesktopPet.app`.

## Herdr consumer contract

This repository does not build or copy `DesktopPet.app`, native packages or character assets. Its integration-only manifest, wrappers and adapter live here; native source/artwork/tooling belong to core. Current `core-release.json` / `release-evidence/` contain reviewed actual Core v0.4.2 authority. The pinned installer SHA-256 is `a0ec644c3baecc7b81baddd29fbbbd3a8f25777e804a6d9940b78820bacc08ec`; [exact installer/checksum/API evidence and limits](docs/releases/v0.4.1.md#actual-core-v042-authority-and-limits) distinguish publication proof from runtime acceptance. Default consumer scripts require that actual v0.4.2 authority; links and local source alone are not proof.

The consumer discovers the exact compatible shared native installation in this order: explicit absolute executable → validated account-home shared locator → native `desktop-pet` on PATH → actual Brew prefix → account-home `Applications/DesktopPet.app`. Invalid explicit selection or locator fails closed. Only an absent installation permits bootstrap from the pinned, digest-verified **core** installer; incompatible installs require explicit native upgrade/repair, never a product build or silent fallback here.

Herdr profile precedence is injected `HERDR_PLUGIN_CONFIG_DIR` / `HERDR_PLUGIN_STATE_DIR` → explicit `--config-dir` / `--state-dir` → Herdr XDG/HOME defaults. The wrapper passes both resolved paths explicitly to the core; it never moves or merges profiles. Backend provenance uses `--herdr-socket ABS`, not legacy `--socket` or the pet's private control socket. `status` identifies the actual running image/profile, not merely the selected CLI.

The [Herdr guide](integrations/herdr/README.md) retains all 21 action IDs and their visibility/bubble/lifecycle semantics, host build/patch, link and OMP instructions. `ensure` is idempotent and does not reveal a hidden pet. Optional attach auto-start requires a real hook-capable host. Fresh absent lifecycle files default Herdr exit policy to true; an existing file missing that key defaults false. Explicit policy and unknown fields survive writes. Tern is a separate consumer and never auto-starts the shared app.

## Source / link

Install or build the core first (`bash scripts/install.sh --source` in the **desktop-pet core** checkout). Then link **this integration checkout**, not the core, using the selected compatible app. `plugin link` does not run a build. Actual v0.4.2 consumer pin materialization is complete in this candidate; corrected public integration/bootstrap acceptance still awaits the parent's gates and publication. See [separate checkouts](integrations/herdr/README.md#build-in-separate-checkouts); plugin installation does not patch Herdr.

Common [usage](https://github.com/hanbong5938/desktop-pet/blob/main/docs/usage.md), [CLI](https://github.com/hanbong5938/desktop-pet/blob/main/docs/cli.md), [development](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.md) and native feature/provider details belong to the independent core, not this source tree.

## Ownership

This repository owns `native-adapter`, `herdr-plugin.toml`, thin install/run wrappers, `integrations/herdr` and `integrations/omp`, integration validation, and frozen historical releases. App/UI/rig/artwork/packaging/source instructions belong to [desktop-pet](https://github.com/hanbong5938/desktop-pet). `.herdrchar`, pack v5, `default@0`, rig ABI and [herdr-characters](https://github.com/hanbong5938/herdr-characters) catalog/gallery authority are unchanged. All published release bodies/tags/assets remain immutable. Corrected v0.4.1 source must pass whole working/index and archived-tree ownership gates and carries no native bundle, archive or rig product.

## Artwork and licensing

[Software license](LICENSE.txt) and artwork/model terms are **distinct**: MIT software terms do not relicense character art. Bundled Rubelia's [license](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/LICENSE.txt), [attribution](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/ATTRIBUTION.txt), [source record](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/source-record.json), and [background-repair record](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/background-repair.json) remain authoritative. `LICENSE.txt` also retains the original Coding Cat notice; its artwork/source moved to the [character repository](https://github.com/hanbong5938/herdr-characters). Keep each optional pack's own terms with it. Artwork-owner approval does not grant model rights; Qwen's research license is an authoring record, and model engines/weights are not bundled. See [character authoring and licenses](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.md#character-authoring).
