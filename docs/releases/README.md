# Herdr Desktop Pet versions

**English** · [한국어](README.ko.md)

Reviewed Markdown in this repository is the canonical version documentation. The GitHub Wiki is a generated reading view, not a separately edited source. These pages describe versions; use the [root overview and installation](../../readme.md), [current usage guide](../usage.md), [development guide](../development.md), and [Herdr integration guide](../../integrations/herdr/README.md) for their respective procedures.

## Choose the right source

| What you need | Where to go |
| --- | --- |
| Current app controls, characters, observation and troubleshooting | [Usage guide](../usage.md) · [CLI contract](../cli.md#english) |
| Source builds, character authoring and the promotional website | [Development guide](../development.md) |
| Published v0.3.6 stable, Tern integration, four languages and protocol-3 beta opt-in | [v0.3.6 canonical release note and upgrade guidance](v0.3.6.md) · [0.3 publication evidence](0.3.md#observed-v036-publication) |
| v0.2.1 public stable compatibility patch, Arin archive import | [0.2 release line](0.2.md) · [v0.2.1 bilingual notes](v0.2.1.md) · [v0.1.11 → v0.2.0 migration (English/한국어)](../migrations/v0.1.11-to-v0.2.0.md) |
| Historical published 0.1 binaries, last at v0.1.11 | [0.1 release line](0.1.md) · [v0.1.11 Release](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/v0.1.11) |
| Other historical transition: composer to inline reply | [v0.1.4 → v0.1.6 migration](../migrations/v0.1.4-to-v0.1.6.md) |
| Version and compatibility rules | [Versioning and compatibility](policy.md) |
| How reviewed notes reach Wiki and new Release bodies | [Publishing](publishing.md) |
| Current public release/asset state | [GitHub Releases](https://github.com/hanbong5938/herdr-desktop-pet/releases); the generated Wiki's **Release-Status** and **Older-Releases** pages |

**v0.3.6 is public stable, non-draft, non-prerelease and GitHub's observed `latest`.** Its [exact Release](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/v0.3.6) has all four assets; [current publication and consumer evidence](0.3.md#observed-v036-publication) records anonymous download verification, the archive SHA-256, the published stable tap cutover, an actual v0.3.5 → v0.3.6 Homebrew upgrade and a pinned-prebuilt install without Cargo/Bun/Node/npm. The frozen [canonical note](v0.3.6.md) covers Tern integration, all four languages and protocol-3 beta opt-in; its pre-publication wording remains historical. The consumer route installs the native prebuilt app with `brew install hanbong5938/tap/herdr-desktop-pet`, without Rust, then explicitly installs the Tern connector with `tern plugin install https://github.com/hanbong5938/tern-desktop-pet`. The same native app defaults to Herdr; Tern requires explicit `--host tern` startup and separate config **and** state paths as shown in the [usage guide](../usage.md#tern-installation-and-separate-profile). The plugin does not silently download, build or execute the app. Consumer smoke exercised native Tern-only startup and empty-snapshot exchange; the actual public plugin CLI install proves discovery, not a GUI window connection or populated-agent delivery. The [previous v0.3.5 publication record](0.3.md) and all old beta records remain historical.

The generated Wiki derives publication status from GitHub's actual Release catalog and complete uploaded assets, **not** from a Markdown file or tag. The linked v0.3.6 evidence is an observed publication record, not a continuously refreshed API report. In the historical 0.1 line, **v0.1.5 is a tag only, not a published binary release**.

## Release lines

| Line | Notes and upgrade context |
| --- | --- |
| [0.3](0.3.md) | Published stable v0.3.6 [publication evidence](0.3.md#observed-v036-publication) and frozen [canonical note/upgrade guidance](v0.3.6.md); previous v0.3.5 and separate beta evidence remain historical and immutable |
| [0.2](0.2.md) | Historical v0.2.1 stable compatibility patch [canonical notes](v0.2.1.md); v0.2.0 [stable notes](v0.2.0.md) and [bilingual upgrade guidance](../migrations/v0.1.11-to-v0.2.0.md) |
| [0.1](0.1.md) | Historical published v0.1.4 and v0.1.6–v0.1.11; tagged-only v0.1.5 |

Future minor lines get their own `X.Y.md` overview with tagged version notes and migration guidance as needed. This index does not promise a support lifetime.

## Publication and historical evidence

For new releases, reviewed `docs/releases/vX.Y.Z.md` at the exact release tag supplies both the generated Wiki version page and the new GitHub Release body. See [Publishing](publishing.md) for the procedure and [policy](policy.md) for the compatibility contract.

The v0.1.4–v0.1.11 tags predate this documentation tree. Their notes are explicitly reconstructed from contemporaneous evidence in the reviewed current source, with links to historical tagged sources and actual Releases where available. That exception does not rewrite old tags, archives, or Release bodies. Historical verification statements describe previously reported evidence, not checks run while creating these pages.
