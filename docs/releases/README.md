# Herdr Desktop Pet versions

**English** · [한국어](README.ko.md)

Reviewed Markdown in this repository is the canonical version documentation. The GitHub Wiki is a generated reading view, not a separately edited source. These pages describe versions; they do not replace the [installation and runtime guide](../../readme.md) or the [Herdr integration guide](../../integrations/herdr/README.md).

## Choose the right source

| What you need | Where to go |
| --- | --- |
| v0.2.1 stable compatibility patch, Arin archive import and safe upgrade | [0.2 release line](0.2.md) · [v0.2.1 bilingual notes](v0.2.1.md) · [v0.1.11 → v0.2.0 migration (English/한국어)](../migrations/v0.1.11-to-v0.2.0.md) |
| Historical published 0.1 binaries, last at v0.1.11 | [0.1 release line](0.1.md) · [v0.1.11 Release](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/v0.1.11) |
| Other historical transition: composer to inline reply | [v0.1.4 → v0.1.6 migration](../migrations/v0.1.4-to-v0.1.6.md) |
| Version and compatibility rules | [Versioning and compatibility](policy.md) |
| How reviewed notes reach Wiki and new Release bodies | [Publishing](publishing.md) |
| Current public release/asset state | [GitHub Releases](https://github.com/hanbong5938/herdr-desktop-pet/releases); the generated Wiki's **Release-Status** and **Older-Releases** pages |

**Source and publication authority differ.** v0.2.1 is a narrow patch of the public v0.2.0 stable source; the current `main` checkout also contains unrelated unreleased features that are **not** part of this patch. `bash scripts/install.sh --source` builds the checkout you actually have. The default/prebuilt installer must point to v0.2.1 only after its published assets and installer update are available. The separate beta1/beta2/beta3 tags and assets remain historical. The [v0.2.1 note](v0.2.1.md) describes the compatibility fix, while the [v0.2.0 note](v0.2.0.md) and [migration guide](../migrations/v0.1.11-to-v0.2.0.md) describe the underlying stable behavior; the [older Unreleased record](unreleased.md) describes prior local/beta provenance, not this patch.

The generated Wiki derives publication status from GitHub's actual Release catalog and complete uploaded assets, **not** from a Markdown file or tag. This source index is not a live API-status report and does not by itself establish that the stable archive was published. In the historical 0.1 line, **v0.1.5 is a tag only, not a published binary release**.

## Release lines

| Line | Notes and upgrade context |
| --- | --- |
| [0.2](0.2.md) | v0.2.1 stable compatibility patch [canonical notes](v0.2.1.md); v0.2.0 [stable notes](v0.2.0.md) and [bilingual upgrade guidance](../migrations/v0.1.11-to-v0.2.0.md); consult the Release catalog for actual publication |
| [0.1](0.1.md) | Historical published v0.1.4 and v0.1.6–v0.1.11; tagged-only v0.1.5 |

Future minor lines get their own `X.Y.md` overview with tagged version notes and migration guidance as needed. This index does not promise a support lifetime.

## Publication and historical evidence

For new releases, reviewed `docs/releases/vX.Y.Z.md` at the exact release tag supplies both the generated Wiki version page and the new GitHub Release body. See [Publishing](publishing.md) for the procedure and [policy](policy.md) for the compatibility contract.

The v0.1.4–v0.1.11 tags predate this documentation tree. Their notes are explicitly reconstructed from contemporaneous evidence in the reviewed current source, with links to historical tagged sources and actual Releases where available. That exception does not rewrite old tags, archives, or Release bodies. Historical verification statements describe previously reported evidence, not checks run while creating these pages.
