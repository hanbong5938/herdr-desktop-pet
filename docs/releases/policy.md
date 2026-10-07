# Versioning and compatibility

[Versions](README.md) · [한국어 버전 안내](README.ko.md) · [Publishing](publishing.md)

## A commit is not a release

The original working-copy app manifests remain **0.1.11**. [Unreleased](unreleased.md) separates the default-branch documentation/tooling change from four native feature groups implemented on local `worktree/rapid-harbor-d2a6` at `bf23ced1649fd0074aec8736644c8f02aa0c492b`. Later working-copy additions and the reply-composer/Markdown scanner corrections are also included with those groups in the selected beta2 snapshot, not in `main` or stable v0.1.11. Merging documentation does not merge that branch; building `main` with `--source` does not acquire its native features. A commit does not ship a binary, change the manifest pin, publish assets, or prove an installed app has that behavior. The original local branch/commit identifies existing implementation provenance, not a guaranteed public download.

A tag alone is also not a published binary release. v0.1.5 is the historical example: packaging failed before assets were published. The generated Wiki determines public release status from the GitHub catalog: a stable, non-draft release must have an actual publication timestamp and all required nonempty, uploaded app archive/checksum assets. A note file, a draft, or partial assets are not a substitute.

## Pre-1.0 version choices

- **Patch (`0.Y.Z`)**: contract-preserving fixes. Correct an implementation or distributed asset while retaining the documented capability, behavior, compatibility, and consumer-safety contract.
- **Minor (`0.Y.0`)**: capability additions, intended behavior changes, or compatibility changes while the app is below 1.0. Describe the old and new contract and provide migration guidance for affected consumers; a pre-1.0 version is not permission to leave changes unexplained.
- Evaluate the actual change, not its size or the name of the affected file. A fix that changes a documented consumer contract needs the minor-change treatment rather than being hidden as a patch.

These are forward-looking release decisions, not a claim that historical releases already followed this policy. The [0.1 overview](0.1.md) records earlier changes, including the v0.1.4 → v0.1.6 reply transition.

**Original working-copy manifests remain 0.1.11.** Independent visibility, selectable menu-bar behavior and images, and linked-worktree removal add capabilities, meeting the pre-1.0 minor criterion above. The [published beta1 prerelease](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/beta%2F0.2.0-beta.1) and its tag `beta/0.2.0-beta.1` are immutable; its frozen source commit `6749439657f73ea6882b769f4a9968f637f8d6d4` and shipping verification are **beta1-only**. The separately selected isolated beta2 snapshot uses app version `0.2.0-beta.2` and tag `beta/0.2.0-beta.2` and includes the earlier beta features plus reply-input clipping and Markdown scanner corrections. This is not stable `0.2.0`: public `main`, stable v0.1.11/latest, stable Homebrew formula, and default installation stay on stable; only the separate beta formula is intended to move to beta2. A beta tag alone is not beta2 publication evidence: verify the GitHub prerelease record and uploaded assets independently.

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

For a **new stable** release, `docs/releases/vX.Y.Z.md` is reviewed in the main repository before tagging. Its first heading is exactly `# Herdr Desktop Pet vX.Y.Z release notes`. The release tag freezes that note. The new GitHub Release body and generated Wiki version page use the same tagged source; repository-relative links in Release bodies resolve to that exact tag, while Wiki links resolve to generated pages or explicit source links. Version notes do not maintain a second handwritten release date; GitHub's release catalog owns publication state and timestamp. The beta2 canonical `docs/releases/v0.2.0-beta.2.md` belongs **only to the isolated beta snapshot**; do not add its prerelease filename to the original stable documentation tree, whose Wiki scan handles stable notes. The beta tag does not trigger stable `v*` Release/Wiki workflows; beta2 publication uses its frozen note and separate prerelease channel.

The Wiki's navigation and status use the current release catalog, not the catalog as it might have looked at an old tag. Unreleased and current navigation use reviewed default-branch documentation; guidance for the local pending native branch remains explicitly scoped to that checkout rather than represented as default-branch app behavior. The authorized main publication contains documentation/tooling only and leaves native, plugin and integration code unchanged. See [Publishing](publishing.md) for generation and publication details.

### Historical documentation exception

The initial v0.1.4–v0.1.11 tags predate the canonical release-note tree. Their pages are reconstructed in reviewed current source from contemporaneous evidence, with provenance and links to the original tags and actual Releases. v0.1.5 has a source tag but no binary download. Generating these historical pages does not retag, replace archives, or rewrite old Release bodies. For future releases whose tagged notes exist, use the frozen tagged content instead of silently replacing it with later edits.

Historical test counts, native probes, and limitations remain reported evidence with their original scope. They are not newly executed verification, and a source probe is not proof of publication, physical keyboard IME behavior, or OS-granted focus transfer.

## Release lines are navigation, not support promises

Each minor line has an `X.Y.md` overview and individual `vX.Y.Z.md` notes; add a new line to the bilingual versions index when it exists. Migration guides explain consumer-visible transitions. Keeping an old release link available is not a promise of backports, ongoing testing, a maintenance window, LTS, or an EOL date. No LTS/EOL guarantees or invented support labels are introduced here.

## 한국어 핵심 원칙

커밋·태그만으로 배포 완료가 되지 않습니다. 원본 작업본 매니페스트는 0.1.11이며 공개 `main`·안정판 v0.1.11·latest·안정판 Homebrew 포뮬러·기본 설치는 그대로입니다. 독립 표시·메뉴 막대 모드/이미지·연결 워크트리 삭제는 기능 추가이므로 1.0 이전 minor 기준에 해당합니다. 게시된 `beta/0.2.0-beta.1` 태그와 동결 소스 `6749439657f73ea6882b769f4a9968f637f8d6d4`, 검사 근거는 **변경할 수 없는 beta1 기록**입니다. 선택한 별도 격리 beta2 스냅샷만 `0.2.0-beta.2` 앱 버전과 `beta/0.2.0-beta.2` 태그를 사용하며 앞선 기능과 답장 입력 글자 잘림·Markdown 스캐너 수정을 포함합니다. 안정판 `0.2.0`은 아직 출시하지 않았으며 beta2 태그나 문서만으로 게시 완료를 주장할 수 없습니다. 베타 포뮬러만 별도로 갱신하고 실제 GitHub prerelease 기록·자산을 확인해야 합니다. beta2 공식 기록은 격리 소스의 버전 문서에만 두며 원본 안정판 Wiki 문서 트리에는 사전 출시 파일을 만들지 않습니다. 원래 네 네이티브 변경은 로컬 `worktree/rapid-harbor-d2a6`의 `bf23ced1649fd0074aec8736644c8f02aa0c492b`에서 시작했고 공개 `main`의 소스 빌드나 v0.1.11에 포함되지 않습니다. patch는 기존 계약을 보존하는 수정이고 minor는 기능·동작·호환성 변경 및 마이그레이션 안내에 사용합니다.

앱 버전, Herdr 호스트의 실제 기능, 캐릭터 팩 v5, 내장 `default@0`는 서로 다른 기준입니다. 호스트 최소 버전만 보고 `client.attached`나 프롬프트 API 지원을 가정하지 마세요. 원격 관찰은 읽기 전용이고 로컬 프롬프트 접수 확인은 에이전트 작업 완료가 아닙니다. 설치·실행은 [한국어 루트 안내](../../readme.ko.md), 로컬 pending 브랜치 전환은 [한국어 업그레이드 안내](../migrations/unreleased.ko.md)를 따르세요. 릴리스 계열을 보관해도 LTS·EOL·백포트·지원 기간을 약속하지 않습니다.
