# Versioning and compatibility

[Versions](README.md) · [한국어 버전 안내](README.ko.md) · [Publishing](publishing.md)

## A commit is not a release

The app remains **0.1.11**. [Unreleased](unreleased.md) separates the default-branch documentation/tooling change from four native feature commits that remain local and unmerged on `worktree/rapid-harbor-d2a6` at `bf23ced1649fd0074aec8736644c8f02aa0c492b`. Neither `main` nor the published v0.1.11 binary implements those four pending features. Merging documentation does not merge that branch; building `main` with `--source` does not acquire its features. A commit does not ship a binary, change the manifest pin, publish assets, or prove an installed app has that behavior. The local branch/commit identifies an existing checkout, not a publicly fetchable source or download.

A tag alone is also not a published binary release. v0.1.5 is the historical example: packaging failed before assets were published. The generated Wiki determines public release status from the GitHub catalog: a stable, non-draft release must have an actual publication timestamp and all required nonempty, uploaded app archive/checksum assets. A note file, a draft, or partial assets are not a substitute.

## Pre-1.0 version choices

- **Patch (`0.Y.Z`)**: contract-preserving fixes. Correct an implementation or distributed asset while retaining the documented capability, behavior, compatibility, and consumer-safety contract.
- **Minor (`0.Y.0`)**: capability additions, intended behavior changes, or compatibility changes while the app is below 1.0. Describe the old and new contract and provide migration guidance for affected consumers; a pre-1.0 version is not permission to leave changes unexplained.
- Evaluate the actual change, not its size or the name of the affected file. A fix that changes a documented consumer contract needs the minor-change treatment rather than being hidden as a patch.

These are forward-looking release decisions, not a claim that historical releases already followed this policy. The [0.1 overview](0.1.md) records earlier changes, including the v0.1.4 → v0.1.6 reply transition.

**No version bump is made by this documentation work.** `0.2.0` is only a candidate for a future minor release if the pending changes are selected and reviewed for shipment. It is not an assigned next version, release commitment, schedule, tag, or download. Unreleased stays unversioned until that decision is made.

## Keep compatibility dimensions separate

| Dimension | Meaning and authority |
| --- | --- |
| Desktop app version | App behavior and its packaged binary; current published baseline is v0.1.11. See the individual release notes and root [runtime guide](../../readme.md). |
| Herdr host version and capabilities | Independent host implementation. Local observation has a 0.9.0+ baseline; remote observation needs compatible saved-machine forwarding and server support; `agent.prompt` needs actual server method support. Client-attach auto-start requires official 0.9.3 source with the supplied patch applied and built, or a future actual host release advertising that hook. Stock 0.9.0/0.9.3 do not provide `client.attached`. A manifest minimum does not prove compatibility. See the [integration guide](../../integrations/herdr/README.md). |
| Character pack format | **v5** is the character-pack format, not the app version or a Herdr host release. Existing authoring/pack guides and validation rules remain authoritative; app documentation does not silently change this format. |
| Bundled default identity | **`default@0`** identifies the built-in Rubelia default. Artwork corrections do not by themselves imply a new app compatibility level, pack-format version, or imported-character selection. Existing artwork licenses, attribution, source records, and approval evidence remain authoritative. |

An app release does not release Herdr, install a patched host, guarantee a host API method, or relicense artwork. The installation/runtime authority stays in the bilingual root [English](../../readme.md) / [Korean](../../readme.ko.md) guides. This policy is not a second installation guide.

Remote observation remains read-only; prompt submission is scoped to an eligible selected local session. Herdr acknowledgement is not agent completion. Compatibility and upgrade notes must preserve those distinctions, IME/send safety, lifecycle settings, and persistence limits rather than reducing them to a version-number table.

## One reviewed source, two publication views

For a **new** release, `docs/releases/vX.Y.Z.md` is reviewed in the main repository before tagging. Its first heading is exactly `# Herdr Desktop Pet vX.Y.Z release notes`. The release tag freezes that note. The new GitHub Release body and generated Wiki version page use the same tagged source; repository-relative links in Release bodies resolve to that exact tag, while Wiki links resolve to generated pages or explicit source links. Version notes do not maintain a second handwritten release date; GitHub's release catalog owns publication state and timestamp.

The Wiki's navigation and status use the current release catalog, not the catalog as it might have looked at an old tag. Unreleased and current navigation use reviewed default-branch documentation; guidance for the local pending native branch remains explicitly scoped to that checkout rather than represented as default-branch app behavior. The authorized main publication contains documentation/tooling only and leaves native, plugin and integration code unchanged. See [Publishing](publishing.md) for generation and publication details.

### Historical documentation exception

The initial v0.1.4–v0.1.11 tags predate the canonical release-note tree. Their pages are reconstructed in reviewed current source from contemporaneous evidence, with provenance and links to the original tags and actual Releases. v0.1.5 has a source tag but no binary download. Generating these historical pages does not retag, replace archives, or rewrite old Release bodies. For future releases whose tagged notes exist, use the frozen tagged content instead of silently replacing it with later edits.

Historical test counts, native probes, and limitations remain reported evidence with their original scope. They are not newly executed verification, and a source probe is not proof of publication, physical keyboard IME behavior, or OS-granted focus transfer.

## Release lines are navigation, not support promises

Each minor line has an `X.Y.md` overview and individual `vX.Y.Z.md` notes; add a new line to the bilingual versions index when it exists. Migration guides explain consumer-visible transitions. Keeping an old release link available is not a promise of backports, ongoing testing, a maintenance window, LTS, or an EOL date. No LTS/EOL guarantees or invented support labels are introduced here.

## 한국어 핵심 원칙

커밋이나 태그만으로 배포 완료가 되지 않습니다. 앱 버전은 0.1.11로 유지합니다. 이번 main 반영은 문서·도구만이며 네이티브·플러그인·통합 코드는 그대로입니다. 네 가지 네이티브 변경은 로컬 `worktree/rapid-harbor-d2a6`의 `bf23ced1649fd0074aec8736644c8f02aa0c492b`에 병합되지 않은 채 남습니다. main의 `--source` 빌드와 공개 v0.1.11에는 포함되지 않으며, 이 브랜치·커밋 표기는 공개 fetch나 다운로드를 보장하지 않습니다. Unreleased에서 두 범위를 구분합니다. 0.2.0은 향후 minor 후보일 뿐 다음 버전 지정이나 출시 약속이 아닙니다. patch는 기존 계약을 보존하는 수정, 1.0 이전 minor는 기능·동작·호환성 변경과 그에 필요한 마이그레이션 안내에 사용합니다.

앱 버전, Herdr 호스트의 실제 기능, 캐릭터 팩 v5, 내장 `default@0`는 서로 다른 기준입니다. 호스트 최소 버전만 보고 `client.attached`나 프롬프트 API 지원을 가정하지 마세요. 원격 관찰은 읽기 전용이고 로컬 프롬프트 접수 확인은 에이전트 작업 완료가 아닙니다. 설치·실행은 [한국어 루트 안내](../../readme.ko.md), 로컬 pending 브랜치 전환은 [한국어 업그레이드 안내](../migrations/unreleased.ko.md)를 따르세요. 릴리스 계열을 보관해도 LTS·EOL·백포트·지원 기간을 약속하지 않습니다.
