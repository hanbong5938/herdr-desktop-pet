# Herdr integration development

**English** · [한국어](development.ko.md) · [Integration overview](../readme.md)

The native app, UI, rig, character tools, artwork, build, packaging and app website moved to [Desktop Pet core](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.md). Build/install the app in that separate core checkout; this integration never builds or copies it. Do not link the core as a Herdr plugin.

[The retained integration guide](../integrations/herdr/README.md) is authoritative for the 21 actions, adapter lifecycle, exact profile/discovery precedence, separate host/core/integration checkouts, official Herdr 0.9.3 patch, optional client-attach hook, disposable host session and OMP extension. `herdr plugin link /absolute/path/to/herdr-desktop-pet` links this consumer checkout and does not build native. Stock Herdr 0.9.0/0.9.3 do not implement the attach hook; installing a plugin never patches the host.

- [Core app development / installation](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.md) · [CLI](https://github.com/hanbong5938/desktop-pet/blob/main/docs/cli.md)
- [Core character authoring and licensing](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.md#character-authoring)
- [Tern development / Connect / Configure](https://github.com/hanbong5938/tern-desktop-pet)
- [Legacy migration](https://github.com/hanbong5938/desktop-pet/blob/main/docs/migrations/desktop-pet.md)

## Integration landing site

This repository's static `site/` contains only four-locale integration/migration entrypoints, no native app UI or character assets. Its Pages workflow stages `site/` directly, preserving pinned Actions and main-only deployment. No Node/Bun app website tooling is needed here. Common app Pages tooling and Wiki rendering/publication belong to core: [core Pages guide](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.md#website-github-pages) and [core Wiki publication](https://github.com/hanbong5938/desktop-pet/blob/main/docs/releases/publishing.md). Pages configuration and live URLs are owner-verified separately; source changes are not deployment proof.

Actual Core Pages deployed `cb10ce2` in [run 38493874667](https://github.com/hanbong5938/desktop-pet/actions/runs/38493874667), with anonymous EN disclosure and KO installation surfaces observed. Herdr's public thin site remains historical `f65f123`; this candidate has not been deployed. The Core Release job succeeded, but the dependent Wiki job in [run 38493874291](https://github.com/hanbong5938/desktop-pet/actions/runs/38493874291) failed for missing dedicated `WIKI_PUBLISH_TOKEN`. Full catalog rendering on Linux succeeded; it is not an initialized Home or a Wiki push. Do not reuse broad `gh`/`GITHUB_TOKEN` as the missing dedicated Wiki credential.

## Whole-source ownership gate

Run `bash scripts/verify-integration.sh` from the real Git checkout. It checks the entire working source and Git index before the disposable overlay, not just the copied files. `/dist/` is ignored and obsolete common source/assets/tooling, tracked generated products, native bundles/archives/binaries/character/rig sidecars and renamed native/archive bytes are rejected. Ignored-but-tracked files are still checked; local dependency/adapter build storage is never overlaid. Host patches, OMP source and authentic installer/release text evidence remain integration-owned.

Before the parent creates a new tag, `bun scripts/integration-release.ts tree REVIEWED_HERDR_SHA` checks the complete archived Git tree directly; publication repeats it. Historical v0.4.0 remains unchanged despite its `dist/HerdrDesktopPet.app` fault. Actual published Core v0.4.2 source `cb10ce2c3cc65402cbb914d60301f7627d5546d5`, release `412241744`, installer SHA-256 `a0ec644c3baecc7b81baddd29fbbbd3a8f25777e804a6d9940b78820bacc08ec` has been anonymously materialized and reviewed into current metadata/evidence; v0.4.1 evidence is history only. The actual source/TypeScript/105 adapter gate passed against contracts `85c644cd90d8c6a9e97e9dc313cc4750f344235f`, including independently exercised native-worker/symlink-index ownership guards. [Corrected v0.4.1](releases/v0.4.1.md) is still untagged/unpublished, pending final parent gates; no final Herdr ID/SHA is known. See the [publication sequence](releases/publishing.md#corrected-v041-parent-publication-sequence).

[Frozen v0.3.6 development documentation](https://github.com/hanbong5938/herdr-desktop-pet/blob/v0.3.6/docs/development.md) records historical builds only. The parent observed real owned v0.4.1 → v0.4.2 Brew upgrade, signature/receipt/derivation validation, `brew test` and published-wrapper same/default-profile Auto UI/control readiness at PID `6863` with zero Herdr/Tern hosts. That scoped proof is not standalone native-update, bridge retirement or two-host completion. Real Tern keychain-provider and OMP model SDK prerequisites remain unavailable; no reconnect/model delivery is asserted.
