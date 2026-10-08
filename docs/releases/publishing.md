# Publishing release documentation and the Wiki

[Versions](README.md) · [Versioning and compatibility](policy.md) · [Unreleased](unreleased.md) · [한국어 버전 안내](README.ko.md)

Reviewed Markdown in the repository is canonical. GitHub Wiki is a generated reading view, not a second editing authority. For **new** app releases, the Release body and Wiki version page use the same note frozen by the release tag. Current indexes, policy, migrations and Unreleased use reviewed default-branch documentation and the live GitHub release catalog. The reviewed [v0.3.0 note](v0.3.0.md) documents the integrated stable minor; neither it nor a source build proves publication, which requires the exact non-draft Release and complete uploaded assets. The [v0.2.1 patch note](v0.2.1.md) remains frozen at the separate public v0.2.0-based compatibility patch and must not be retroactively rewritten to include the newer features.

The original documentation/tools-only authorization, v0.2.0 promotion and narrower v0.2.1 patch authorization below are **historical release scopes**, not a continuing ban on the separately authorized v0.3.0 minor source. Preserve beta1/beta2/beta3 and `beta/0.3.0-beta.2` tags/assets, the separate beta formula, pack v5, bundled `default@0`, artwork/rig overrides/motion/licensing and the independent Herdr host.

## Authorization and setup record

The original **read-only setup inspection** found owner ADMIN access, Wiki disabled and no repository Actions secrets; it made no setting, secret or publication changes. Later Wiki activation created initial HEAD `680633c28f33f264b5d532a385370e1ec5995d1a`. That historical inspection is not current credential status: [the v0.2.1 release run](https://github.com/hanbong5938/herdr-desktop-pet/actions/runs/37787113731) successfully generated 24 managed Wiki pages and pushed Wiki commit `b0d63dc` using its supplied `WIKI_PUBLISH_TOKEN`. This patch operation did not create or change any secret. Credentials must never be printed, persisted in source or copied from a broad owner login into a CI secret.

The original documentation/tools-only publication used the then-main source and excluded four local native commits (`078f5c2`, `e828c77`, `f638f0c`, `bf23ced`). That is historical provenance. The isolated v0.2.1 patch was tagged from v0.2.0-based source, not the then-unreleased `main`; its Wiki version page and Release body remain frozen there. The v0.3.0 release instead needs its own integrated source/tag and reviewed [new note](v0.3.0.md), with current navigation from the reviewed pushed default branch.

### Historical authorized v0.2.1 patch and local Wiki publication

1. Review the isolated patch source/docs based on public v0.2.0 (`b8b7b8c`), including the focused reference-only mesh fix, version/installer cutover and [v0.2.1 canonical note](v0.2.1.md). Do not mix in unrelated unreleased `main` features. Integrate/review current documentation on the default branch separately; the v0.2.1 version page and Release body must always use the exact patch tag.
2. Validate the stable `v0.2.1` tag and canonical note, publish the actual arm64 app archive, `.sha256` and `SHA256SUMS` through the Release workflow, then update the stable Homebrew formula/default installer to v0.2.1 as authorized. Do not alter archived v0.2.0 or beta tags/assets or the beta formula. A completed tag or source build alone is not binary-publication proof; confirm actual uploaded assets/checksums and consumer upgrade separately.
3. Wiki is enabled with initial Home and Git HEAD. Render current navigation from the reviewed pushed default branch and the version page from the frozen v0.2.1 tag using the live Release catalog, then publish with a usable Wiki-write credential **only** in the local publisher process environment. This does not create an Actions secret or dispatch Actions publication.
4. Confirm the remote Wiki push/result and actual Release/asset publication independently. The Release job may succeed while its dependent Wiki job fails for lack of `WIKI_PUBLISH_TOKEN`; do not rerun the binary Release to repair the Wiki or claim the Wiki published before observing its remote result.

### v0.3.0 stable publication contract

Review the integrated source, four matching manifests/root lock and bilingual [v0.3.0 note](v0.3.0.md) on the final candidate; run combined TypeScript/Bun, native/coordinator, release build/package, signed updater/helper and worker-entitlement checks and isolated updater/UI/recovery acceptance. PR #17's pre-merge source gates and ad-hoc signing are historical scoped evidence, not substituted for these final gates. Freeze the reviewed note at the exact v0.3.0 tag. Publish a non-draft stable Release with arm64 archive, `.sha256` and `SHA256SUMS`; download and verify all assets and their digests. Then update the stable default/`--prebuilt` installer and Homebrew formula and prove a consumer install/upgrade plus restarted daemon `app_version` and executable. Keep prior stable and beta assets unchanged. A tag, prepared note or incomplete Release does not suffice. Verify the Wiki's tagged version page/current navigation push separately; a failed Wiki job is repaired with Wiki-only sync, not a repeat binary release.

### Credential prerequisite for future automation

Before relying on main-push, workflow-dispatch or post-Release Wiki synchronization:

1. Wiki is already enabled and its initial Home is saved; preserve that history.
2. Provision a **dedicated** automation credential as `WIKI_PUBLISH_TOKEN` in this repository's Actions secrets. Its account must have write access to this repository's Wiki, with any organization approval/SSO requirements satisfied. Do not copy the owner's existing broad `gh` OAuth login into an automation secret.
3. Confirm Actions can run the workflows and the ordinary `GITHUB_TOKEN` can read repository metadata, source and the Release API. The Wiki credential is not used for those reads. Until the dedicated secret is provisioned, automatic or workflow-dispatch publication lacks its required push credential; local manual publication does not remove that prerequisite.

Choose a credential type that is actually supported for **Git-over-HTTPS Wiki push**, not merely one that can read the main repository API. A dedicated classic personal access token with `public_repo` for this public repository is one supported option; a private repository would require the corresponding broader `repo` scope. The owning automation account's repository rights still matter. These scopes are broader than a single Wiki, so use a dedicated account/credential, limit account access, set an expiry, record an owner for renewal, and revoke/rotate it if compromised. Do not assume a fine-grained token or a GitHub App installation token works for Wiki Git operations without establishing that support first. The ordinary Actions `GITHUB_TOKEN` is **not assumed to write Wiki**.

Recommended owner safeguards, not runtime prerequisites: protect the default branch and review canonical documentation changes; protect changes to the publisher/workflow; optionally configure an Actions environment with required reviewers and store the token there. If using an environment, add its `environment:` to the Wiki publish job and allow both default-branch runs and intended stable release-tag caller runs. Environment secrets are supplied by the callee job, not forwarded from an unrelated caller environment. The checked-in workflow supports a repository secret without requiring an environment or branch-protection setting.

## Workflow and trust boundary

The [Wiki workflow](../../.github/workflows/wiki.yml) uses the reviewed pushed default-branch SHA for **current** navigation and the exact fetched release tag for a **frozen** version page. For v0.3.0 this must be its integrated stable tag, never the older isolated v0.2.1 patch or a beta tag. It reads the actual default branch and Release catalog; an old tag is not a current-index snapshot. Only default-branch push/manual runs or matching release-tag push callers are accepted. Successful publication still requires the configured Wiki-write credential; the v0.2.1 Wiki run below is historical.

The checkout does not persist credentials. GitHub API reads use only the ordinary `GITHUB_TOKEN` (`contents: read`). Rendering finishes and validates the full managed page set **before** the publisher is invoked. `WIKI_PUBLISH_TOKEN` is assigned only in the publishing step's environment. The workflow uses SHA-pinned actions and one repository-wide, non-cancelling publication concurrency group, shared by push, manual and reusable calls.

The workflow's current push filter is `main`, the repository's current default branch; the checkout itself discovers the actual default branch. If the owner later renames the default branch, update that trigger filter in reviewed source as well.

### Reusable Release integration

The Release workflow validates `docs/releases/vX.Y.Z.md` from the tagged checkout before publishing a new Release. Its first heading must be exactly `# Herdr Desktop Pet vX.Y.Z release notes`, the note must be nonempty, and the package manifest, native Cargo manifest, plugin manifest and the app's root package entry in `native/Cargo.lock` must match the stable tag. `scripts/release-docs.ts check` writes the note body with relative repository file links rewritten to exact tag blob URLs and Markdown images to tagged raw-byte URLs; the Release action uses that file, not a separate handwritten summary.

After the Release assets/body are published, call the Wiki workflow as a separate dependent job:

```yaml
wiki_sync:
  needs: release
  uses: ./.github/workflows/wiki.yml
  permissions:
    contents: read
  with:
    required_release: ${{ github.ref_name }}
  secrets:
    WIKI_PUBLISH_TOKEN: ${{ secrets.WIKI_PUBLISH_TOKEN }}
```

`required_release` is an optional string, defaulting to empty. The reusable secret declaration allows an owner-configured callee environment to provide the credential instead, but the publisher always requires a usable credential for an HTTPS push. Do not add `continue-on-error`: a failed Wiki synchronization must remain visibly failed, even though the already published Release is not rolled back.

The generator requires the selected stable tag to have a **public, non-draft, actually published Release with all three uploaded nonzero required assets**: the app `.tar.gz`, its `.sha256`, and `SHA256SUMS`. Tag existence or a note filename does not meet that requirement. Drafts, incomplete uploads and pending source notes never become shipped entries merely because Markdown was merged.

## Initial historical backfill and future frozen notes

The original v0.1.4–v0.1.11 tags predate `docs/releases`. Their initial Wiki notes come from reviewed current-source reconstructions with contemporaneous evidence and links to old tag sources. This historical exception does not retag, rebuild or rewrite old GitHub Release bodies. v0.1.5 is **tagged only, with no published binary**; it has no download link. Catalog status remains authoritative for every page.

For future published releases, the generator reads `docs/releases/vX.Y.Z.md` from that exact tag. A later main-branch edit cannot silently replace the frozen version page. A published future tag missing its canonical note is an error, not permission to fall back to current prose. Current Home/status/older-release navigation still comes from current source and the current catalog, including when repairing documentation for an older release.

Generated pages contain source attribution with immutable source references. Publication timestamps and completeness come from the API rather than duplicated manual date fields. Historical test counts and probes remain reported evidence; publishing documentation does not execute or recertify them.

## Routine documentation updates and Wiki-only repair

Edit and review canonical pages, then merge to the default branch. Main-branch Wiki synchronization updates navigation and history but does not create a binary Release. The historical v0.2.1 patch used isolated source; v0.3.0 publication requires integrated stable source, its exact tag and separately confirmed assets, stable formula/default-installer cutover and consumer upgrade. Do not count a Wiki-only sync as a release.

After dedicated automation credential setup, repair a failed publication by opening [the Wiki workflow](https://github.com/hanbong5938/herdr-desktop-pet/actions/workflows/wiki.yml), choosing the **default branch**, and selecting **Run workflow**. This Actions manual dispatch is distinct from the authorized local owner-OAuth publication; the Wiki already has initialized Git history:

- Leave **tag** blank for a routine Wiki-only sync/backfill.
- Set **tag** to an existing public complete release, such as `v0.1.11`, when repairing a post-Release failure and you want that publication requirement rechecked.

This manual workflow changes only managed Wiki documentation. It does not rebuild binaries, bump a version, create a tag, publish a new Release, overwrite Release bodies or change repository settings/secrets. Do **not** rerun the app Release job just to repair Wiki.

If a called Wiki job fails, its step summary independently checks the Release API. When the Release is confirmed published, it explicitly says **Release published, but Wiki synchronization failed**, links the surviving Release and this Wiki-only repair workflow. If publication cannot be confirmed, it does not falsely claim the tag shipped. Both cases leave the job failed for inspection.

## Local rendering and isolated Git publication

The generator has no publishing side effects. For an authorized remote Wiki publication, render from the reviewed **pushed default-branch documentation source** at its immutable source SHA with the selected published stable tag fetched for its frozen version note; the isolated v0.2.1 patch source and integrated v0.3.0 stable source are not interchangeable. `git rev-parse HEAD` below must identify that reviewed pushed default-branch snapshot:

```sh
rendered="$(mktemp -d)"
bun scripts/wiki-docs.ts --root . --output "$rendered" \
  --repository hanbong5938/herdr-desktop-pet \
  --source-ref "$(git rev-parse HEAD)"
```

Optional generator arguments:

- `--releases-json FILE`: use a complete, locally supplied GitHub Release catalog JSON array instead of live API reads, for deterministic offline checks.
- `--require-release TAG`: refuse to render for a selected tag that is not public and complete.
- `--root ROOT`, `--output DIR`, `--repository OWNER/REPO`, and `--source-ref REF` identify canonical source, the isolated rendered tree, catalog repository, and a resolvable source reference. In CI the source reference is the immutable checked-out default-branch commit SHA; tag-frozen version pages are resolved separately from fetched tags.

The output includes `.wiki-managed-pages`, a sorted newline-separated list of generated Markdown basenames. [The publisher](../../scripts/publish-wiki.sh) takes that rendered tree directly:

```sh
bash scripts/publish-wiki.sh --source-dir "$rendered" \
  --repository hanbong5938/herdr-desktop-pet \
  --source-ref "$(git rev-parse HEAD)"
```

**The command above really commits and pushes to the GitHub Wiki** and is only for owner-authorized publication from reviewed pushed main. It requires a usable credential as `WIKI_PUBLISH_TOKEN` in the process environment; never put a literal token in command arguments, shell history, files or the remote URL. For **this authorized local manual publication**, the existing owner `gh` OAuth credential may be passed ephemerally to the publisher:

```sh
set +x
WIKI_PUBLISH_TOKEN="$(gh auth token)" \
  bash scripts/publish-wiki.sh --source-dir "$rendered" \
    --repository hanbong5938/herdr-desktop-pet \
    --source-ref "$(git rev-parse HEAD)"
```

Keep tracing disabled before obtaining the credential; do not print or persist it, and never upload it as an Actions secret. This process-local use does not configure or enable future automated publication. For local verification without remote publication, replace the remote explicitly:

```sh
bash scripts/publish-wiki.sh --source-dir "$rendered" \
  --wiki-url /absolute/path/to/an/isolated-initialized-wiki.git \
  --source-ref "$(git rev-parse HEAD)"
```

`--wiki-url` accepts only a local filesystem path or `file:///` URL. It cannot redirect a token-bearing publisher to another network host. The isolated repository must be a real Git remote with an initial commit and valid HEAD; the publisher clones it, discovers its branch and performs a real ordinary push. No token is required for this local mode. For isolated verification, seed an unrelated Wiki page, check its preservation, repeat the command to confirm unchanged content causes no extra commit/push, and exercise an update and failure paths against disposable remotes. Do not treat these scenarios as evidence that remote GitHub Wiki publication has occurred.

Publisher options are exactly `--source-dir DIR` (required), `--repository OWNER/REPO` (defaults to `GITHUB_REPOSITORY`, otherwise `hanbong5938/herdr-desktop-pet`), `--source-ref COMMIT` (optional full 40-character SHA for the commit message), `--wiki-url LOCAL_PATH_OR_FILE_URL` (optional isolated local override), and `--help`. It requires Bash, Git and Bun, and the sibling generator module for shared ownership validation.

## Managed files, credentials and failures

The generator declares the managed Wiki mappings:

| Canonical source | Wiki page |
| --- | --- |
| `docs/releases/README.md`, `README.ko.md` | `Home`, `Home-ko` |
| `policy.md`, `unreleased.md`, `publishing.md` | `Versioning-and-Compatibility`, `Unreleased`, `Publishing` |
| `X.Y.md`, `vX.Y.Z.md` | `Release-Line-X.Y`, `Release-vX.Y.Z` |
| `docs/migrations/v0.1.4-to-v0.1.6.md` | `Migration-v0.1.4-to-v0.1.6` |
| `docs/migrations/v0.1.11-to-v0.2.0.md` | `Migration-v0.1.11-to-v0.2.0` (one bilingual page) |
| `docs/migrations/unreleased.md`, `unreleased.ko.md` | `Upgrade-Unreleased`, `Upgrade-Unreleased-ko`; v0.3.0 bilingual upgrade guidance lives in the supported `docs/releases/0.3.md` overview |
| Catalog/generated navigation | `Release-Status`, `Older-Releases`, `_Sidebar` |

Top-level canonical Markdown filenames in `docs/releases/` and `docs/migrations/` are limited to the mappings above; an unsupported filename fails rendering rather than being ignored. Put manual validation checklists and supporting records in `docs/validation/` instead. Canonical pages may link to those files, but the Wiki renders links as immutable source-commit blob URLs, not additional managed Wiki pages. Checklist procedures are not completed acceptance evidence: keep observed results distinct from unexercised checks. Changes under `docs/validation/` trigger Wiki sync so those pinned links update with the source commit.

Do not manually edit these generated pages; make changes in canonical source. Unrelated Wiki pages are untouched. `.wiki-managed-pages` records the pages the generator owns; only prior-manifest-owned pages that disappear from the new manifest are removed. The publisher validates both manifests against the generator's ownership rules and rejects unexpected rendered files, unsafe paths, symlinks and missing/empty pages. It preserves existing Wiki history and makes at most one content commit per run. Identical rendered content is a no-op.

For HTTPS Git operations, an ephemeral askpass script reads the publish-step environment; its file contains no token. The token is never written to a clone URL, main checkout, Git credential store or commit. Credential helpers and Git tracing are disabled for publication; the temporary clone and askpass script are removed on exit. The remote's HEAD decides the Wiki branch name—`master` or `main` is never assumed. Push is normal, never force.

| Failure | Owner action |
| --- | --- |
| Wiki disabled, repository missing, or Home never saved | Enable Wiki and save initial Home; then use the authorized local publication or, after dedicated credential setup, Wiki-only workflow repair. |
| Missing, expired, revoked or insufficient automation credential | Provision/renew the dedicated token with proven Wiki Git write access and necessary account/SSO rights. Never copy broad owner OAuth into CI; authorized process-local use is not an automation credential. |
| Source ref/tag unavailable, invalid note, missing future frozen note | Fix/review canonical source or fetch the required history; do not invent a release or fall back to unrelated content. |
| Selected Release draft, unpublished or missing required uploaded assets | Inspect the actual Release/asset publication; a note or tag cannot make it shipped. |
| Unsafe/invalid managed manifest or generated destination conflict | Inspect generated ownership metadata and the named Wiki path; preserve unrelated pages while resolving the conflict. |
| Concurrent remote edit or rejected push | Inspect the remote changes, then rerun from fresh current source. Never force-push to erase another edit. |

A render error occurs before the publisher is called. Clone/validation/copy/commit failures do not push a partial generated tree; a rejected push remains a visible error. No retry silently overwrites a moving remote, and no failure mutates app tags, binary assets or Releases.

## 한국어 운영·안전 요약

정본은 저장소에서 검토한 Markdown이고 Wiki는 생성된 읽기용 문서입니다. 공개 v0.2.0 기반의 좁은 **v0.2.1 호환성 패치만** 허용됐던 범위는 과거 패치의 경계이며 별도로 승인된 v0.3.0 minor를 금지하지 않습니다. [v0.3.0 한영 노트](v0.3.0.md)는 통합된 검색·정렬·자동화·초안·크기 조절·공식 다운로드·protocol-2 updater·worker JIT를 설명하지만 게시 증거는 아닙니다. 최종 통합 검사와 실제 비초안 Release·공개 자산/체크섬·설치기/포뮬러·소비자 업그레이드·Wiki 게시를 각각 확인하세요. v0.2.1 노트와 예전 베타 태그·자산은 변경하지 않습니다.

`docs/releases/`와 `docs/migrations/`의 최상위 정본 Markdown 파일명은 위 매핑으로 제한되며, 지원하지 않는 파일명은 무시되지 않고 렌더링 오류를 일으킵니다. 수동 검증 체크리스트와 보조 기록은 `docs/validation/`에 둡니다. 정본 페이지에서 해당 파일을 링크할 수 있지만 Wiki에서는 새 관리 페이지가 아니라 소스 커밋에 고정된 blob URL로 연결됩니다. 체크리스트의 절차 자체는 완료된 인수 검증 증거가 아니므로 관찰한 결과와 실행하지 않은 항목을 구분합니다. `docs/validation/`의 변경도 Wiki 동기화를 트리거하여 고정 링크를 갱신합니다.

원래 읽기 전용 점검에서는 Wiki와 Actions 비밀이 없었고 설정·초기 페이지·원격 게시를 변경하지 않았습니다. 이후 초기 Wiki HEAD `680633c28f33f264b5d532a385370e1ec5995d1a`가 만들어졌습니다. 이 과거 점검을 현재 비밀 설정 상태로 해석하지 마세요. [v0.2.1 릴리스 실행](https://github.com/hanbong5938/herdr-desktop-pet/actions/runs/37787113731)은 공급된 `WIKI_PUBLISH_TOKEN`으로 관리 페이지 24개를 생성하고 Wiki 커밋 `b0d63dc`를 실제 푸시했습니다. 이번 패치 작업은 비밀을 생성·변경하지 않았습니다. 자격 증명을 출력·소스 저장·소유자의 광범위 로그인에서 CI 비밀로 복사하지 말고, 이후 자동화 토큰의 실제 Wiki 쓰기 권한·만료·소유자를 별도로 관리하세요. 일반 `GITHUB_TOKEN`의 Wiki 쓰기 권한은 가정하지 않습니다.

새 릴리스의 본문과 Wiki 버전 페이지는 같은 태그의 고정된 문서를 사용하고, 인덱스·정책·Unreleased는 현재 기본 브랜치 문서와 실제 Release API 상태를 사용합니다. Markdown 병합이나 태그 존재만으로 미배포 변경을 출시 완료로 표시하지 않습니다. 전용 자동화 토큰이 준비된 뒤에는 아래 Wiki 전용 워크플로로 복구할 수 있으며, 허가된 로컬 수동 발행과 Actions의 수동 실행은 다른 경로입니다.

Wiki만 실패했으면 이미 공개된 Release는 그대로 두고 [Wiki 전용 워크플로](https://github.com/hanbong5938/herdr-desktop-pet/actions/workflows/wiki.yml)를 **기본 브랜치**에서 다시 실행하세요. 일반 동기화는 tag를 비우고, 릴리스 후 복구는 해당 공개·완전 배포 태그를 넣습니다. Wiki 복구 때문에 바이너리 릴리스 작업을 다시 실행하지 마세요. 과거 v0.1.5는 태그만 있고 공개 바이너리가 없습니다. 관련 없는 Wiki 페이지와 기존 이력은 보존하고, 충돌 해결에 force push를 쓰지 않습니다. 로컬 `--wiki-url` 검증은 초기 커밋이 있는 격리된 로컬 Git 원격만 대상으로 합니다.
