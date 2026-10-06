# Herdr Desktop Pet versions

**English** · [한국어](README.ko.md)

Reviewed Markdown in this repository is the canonical version documentation. The GitHub Wiki is a generated reading view, not a separately edited source. These pages describe versions; they do not replace the [installation and runtime guide](../../readme.md) or the [Herdr integration guide](../../integrations/herdr/README.md).

## Choose the right source

| What you need | Where to go |
| --- | --- |
| Latest published binary baseline: **v0.1.11** | [Release notes](v0.1.11.md) · [GitHub Release](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/v0.1.11) |
| Documentation and publishing tools on `main`, plus four native feature groups implemented only in an unmerged local checkout | [Unreleased](unreleased.md) · [Local pending-checkout upgrade guidance](../migrations/unreleased.md) |
| Earlier versions in the 0.1 line | [0.1 release line](0.1.md) |
| The composer-to-inline-reply transition | [v0.1.4 → v0.1.6 migration](../migrations/v0.1.4-to-v0.1.6.md) |
| Version and compatibility rules | [Versioning and compatibility](policy.md) |
| How reviewed notes reach Wiki and new Release bodies | [Publishing](publishing.md) |
| Current public release/asset state | [GitHub Releases](https://github.com/hanbong5938/herdr-desktop-pet/releases); the generated Wiki's **Release-Status** and **Older-Releases** pages |

**Source authority matters.** App version remains **0.1.11**. The documentation and publishing tools are on `main`; the four native groups in [Unreleased](unreleased.md) are implemented only in local `worktree/rapid-harbor-d2a6` at `bf23ced1649fd0074aec8736644c8f02aa0c492b`, not in `main` or the public v0.1.11 binary. That checkout is not guaranteed to be publicly fetchable and has no assigned release version or download. An optionless install uses the manifest-pinned prebuilt; `bash scripts/install.sh --source` builds the checkout you already have. On `main`, it does **not** provide those native previews. Use the root source-build instructions for baseline installation, and the pending-checkout guide only if you already have that local implementation.

The generated Wiki derives runtime release status from GitHub's release catalog and complete uploaded assets, not from the existence of a Markdown file or tag. This navigation is not a live API-status report. In particular, **v0.1.5 is a tag only, not a published binary release**.

## Release lines

| Line | Notes and upgrade context |
| --- | --- |
| [0.1](0.1.md) | Published v0.1.4 and v0.1.6–v0.1.11; tagged-only v0.1.5; latest published baseline v0.1.11 |
| [Unreleased](unreleased.md) | Main-branch documentation/publishing changes and separately scoped local, unmerged native changes; no new binary release |

A future minor line gets its own `X.Y.md` overview, linked here, with `vX.Y.Z.md` notes and migration guidance where behavior or compatibility changes. No future line or release is assigned by this index, and no support lifetime is implied.

## Publication and historical evidence

For new releases, reviewed `docs/releases/vX.Y.Z.md` at the exact release tag supplies both the generated Wiki version page and the new GitHub Release body. See [Publishing](publishing.md) for the procedure and [policy](policy.md) for the compatibility contract.

The v0.1.4–v0.1.11 tags predate this documentation tree. Their notes are explicitly reconstructed from contemporaneous evidence in the reviewed current source, with links to historical tagged sources and actual Releases where available. That exception does not rewrite old tags, archives, or Release bodies. Historical verification statements describe previously reported evidence, not checks run while creating these pages.
