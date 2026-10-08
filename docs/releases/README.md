# Herdr Desktop Pet versions

**English** · [한국어](README.ko.md)

Reviewed Markdown in this repository is the canonical version documentation. The GitHub Wiki is a generated reading view, not a separately edited source. These pages describe versions; they do not replace the [installation and runtime guide](../../readme.md) or the [Herdr integration guide](../../integrations/herdr/README.md).

## Choose the right source

| What you need | Where to go |
| --- | --- |
| v0.3.2 stable source, capabilities and safe upgrade | [0.3 release line and bilingual migration](0.3.md) · [v0.3.2 bilingual release note](v0.3.2.md) |
| v0.2.1 public stable compatibility patch, Arin archive import | [0.2 release line](0.2.md) · [v0.2.1 bilingual notes](v0.2.1.md) · [v0.1.11 → v0.2.0 migration (English/한국어)](../migrations/v0.1.11-to-v0.2.0.md) |
| Historical published 0.1 binaries, last at v0.1.11 | [0.1 release line](0.1.md) · [v0.1.11 Release](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/v0.1.11) |
| Other historical transition: composer to inline reply | [v0.1.4 → v0.1.6 migration](../migrations/v0.1.4-to-v0.1.6.md) |
| Version and compatibility rules | [Versioning and compatibility](policy.md) |
| How reviewed notes reach Wiki and new Release bodies | [Publishing](publishing.md) |
| Current public release/asset state | [GitHub Releases](https://github.com/hanbong5938/herdr-desktop-pet/releases); the generated Wiki's **Release-Status** and **Older-Releases** pages |

**Source and publication authority differ.** This checkout selects v0.3.2 as its stable source and default/prebuilt target, retaining the public v0.2.1 compatibility fix for Arin archive imports (app v0.2.1+). Its search/order, automation, drafts, resizing, official character browser, protocol-2 updater and worker JIT belong to v0.3.2, not the old v0.2.1 or immutable beta binaries. `bash scripts/install.sh --source` builds this checkout; successful default/`--prebuilt` installation requires the exact non-draft v0.3.2 Release, complete uploaded assets and updated installer. See the [v0.3.2 note](v0.3.2.md) and [upgrade guidance](0.3.md#upgrade-from-v021-or-beta). The older [Unreleased record](unreleased.md) documents historical source/beta evidence, not public v0.3.2 verification.

The generated Wiki derives publication status from GitHub's actual Release catalog and complete uploaded assets, **not** from a Markdown file or tag. This source index is not a live API-status report and does not by itself establish that the stable archive was published. In the historical 0.1 line, **v0.1.5 is a tag only, not a published binary release**.

## Release lines

| Line | Notes and upgrade context |
| --- | --- |
| [0.3](0.3.md) | v0.3.2 stable source/default-prebuilt target [bilingual notes](v0.3.2.md) and upgrade guidance; consult the Release catalog for actual publication |
| [0.2](0.2.md) | Historical v0.2.1 stable compatibility patch [canonical notes](v0.2.1.md); v0.2.0 [stable notes](v0.2.0.md) and [bilingual upgrade guidance](../migrations/v0.1.11-to-v0.2.0.md) |
| [0.1](0.1.md) | Historical published v0.1.4 and v0.1.6–v0.1.11; tagged-only v0.1.5 |

Future minor lines get their own `X.Y.md` overview with tagged version notes and migration guidance as needed. This index does not promise a support lifetime.

## Publication and historical evidence

For new releases, reviewed `docs/releases/vX.Y.Z.md` at the exact release tag supplies both the generated Wiki version page and the new GitHub Release body. See [Publishing](publishing.md) for the procedure and [policy](policy.md) for the compatibility contract.

The v0.1.4–v0.1.11 tags predate this documentation tree. Their notes are explicitly reconstructed from contemporaneous evidence in the reviewed current source, with links to historical tagged sources and actual Releases where available. That exception does not rewrite old tags, archives, or Release bodies. Historical verification statements describe previously reported evidence, not checks run while creating these pages.
