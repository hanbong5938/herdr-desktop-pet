# Herdr Desktop Pet Unreleased

[Versions](README.md) · [0.1 line](0.1.md) · [Versioning and compatibility](policy.md) · [Upgrade guidance](../migrations/unreleased.md) · [한국어 업그레이드 안내](../migrations/unreleased.ko.md)

**Distinct source authorities are documented here.** The repository documentation and publishing tools are on `main`. The four native feature groups below are implemented only in local `worktree/rapid-harbor-d2a6` at `bf23ced1649fd0074aec8736644c8f02aa0c492b`; they are **unmerged and absent from `main` and the stable public v0.1.11 binary**. The selectable menu-bar mode, custom icon image, and worktree removal described in separate sections below are **current working-copy additions**, not features attributed to that historical commit or to the stable binary. The working-copy app version remains **0.1.11**; the separate, opt-in beta channel below does not change the stable release, working-copy manifests, or `main`. A source commit or Wiki publication does not publish a binary release.

| Source | What it provides |
| --- | --- |
| Public v0.1.11 binary | Published baseline; none of the four pending native groups |
| `main` source | Baseline native implementation plus repository documentation and publishing tools; none of the four pending native groups |
| Local `worktree/rapid-harbor-d2a6` at `bf23ced1649fd0074aec8736644c8f02aa0c492b` | The four implemented but unmerged native groups; guidance only for users who already possess this checkout |
| Current local working copy of `worktree/rapid-harbor-d2a6` | The four pending groups plus separately added selectable menu-bar mode, custom icon image, and worktree removal; none is available from `main` or the public binary |

`bash scripts/install.sh --source` builds the checkout you already have; running it on `main` does **not** install these native previews. The local branch name and commit identify the implementation, not a guaranteed public remote branch, fetchable commit, or download. Follow the bilingual root [English](../../readme.md) / [Korean](../../readme.ko.md) guides for baseline installation and runtime commands. Use the [pending-checkout upgrade guidance](../migrations/unreleased.md) only for that local implementation.

## Optional beta test channel / 선택형 베타 테스트 채널

The user chose to check the actual UI themselves using a manually selected prerelease instead of treating locked-session GUI/IME checks as automated proof. The planned [0.1.12-beta.1 prerelease](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/beta%2F0.1.12-beta.1) uses tag `beta/0.1.12-beta.1`, app version `0.1.12-beta.1`, and `HerdrDesktopPetBeta.app` in `HerdrDesktopPet-v0.1.12-beta.1-macos-arm64.tar.gz` (with `.sha256` and `SHA256SUMS`). The link is usable **once the manual prerelease is published**, not evidence that it is already online. It is built from an isolated snapshot of this current working copy, including both the original four groups and the separate menu-bar/image/worktree-removal additions; only that isolated source has beta-version manifests. It is not a merge into `main`, a change to this working copy's 0.1.11 manifests, or an update to stable v0.1.11, latest, Homebrew, or the default installer. See [English beta installation guidance](../migrations/unreleased.md#optional-manual-beta-test) / [한국어 베타 설치 안내](../migrations/unreleased.ko.md#선택형-수동-베타-테스트). No actual GUI/IME visual check is claimed.

사용자는 잠긴 macOS 세션의 자동 GUI/IME 검증 완료 주장 대신 선택형 베타로 실제 UI를 직접 확인하기로 했습니다. 수동 [0.1.12-beta.1 시험판](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/beta%2F0.1.12-beta.1)의 태그는 `beta/0.1.12-beta.1`, 앱 버전은 `0.1.12-beta.1`이며 `HerdrDesktopPet-v0.1.12-beta.1-macos-arm64.tar.gz` 안의 앱은 `HerdrDesktopPetBeta.app`입니다(`.sha256`, `SHA256SUMS` 동반). **수동 게시 후에만** 링크를 사용할 수 있으며 이미 게시했다는 뜻은 아닙니다. 원래 네 묶음과 현재 작업본의 별도 추가 기능을 모두 포함한 격리 소스 사본에만 베타 버전을 적용합니다. 현재 작업본의 0.1.11 매니페스트·`main`·안정판 v0.1.11·latest·Homebrew·기본 설치 경로는 바꾸지 않습니다. [한국어 수동 설치 안내](../migrations/unreleased.ko.md#선택형-수동-베타-테스트)를 참고하세요. 실제 GUI/IME 시각 검증을 완료했다는 뜻이 아닙니다.

All four pending native groups from the original pending-source record are retained below, separately from the main-branch documentation/tools change. The four historically identified group sections describe the **local checkout only**, not current `main` behavior or shipped binary features. The historical conditional-rescue policy in the second group is superseded **only in the current working copy** by the separately labeled menu-bar visibility and image section after it.

## Compact observation settings

- Separate **This Mac** and **Remote machines** with right-aligned native switches. Nest saved profiles under **Machines to observe**, showing name, session, and status.
- Measure wrapped text for card, row, and scroll heights; remove fixed notice gaps and duplicate help. Reuse controls by opaque profile ID so polling and renaming preserve focus and scroll. Clamp scrolling when the list shrinks.
- Distinguish **initial catalog lookup**, **confirmed empty**, **no selection**, **paused**, **checking**, **observing**, **unavailable**, and **disabled**. Catalog failure retains the previous profile list with an explicit warning; it is not a confirmed empty result. Turning Remote off keeps saved selections and masks stale successful statuses.
- Localize labels and contextual accessibility names in Korean and English. Registration help and folded diagnostics are read-only. Reconnect copies a **shell-quoted command** with native clipboard feedback; it **never executes** that command.

## Contextual settings and conditional rescue

- Right-click or Control-click the character, bubble background, or card header for **Settings…** in the same full three-tab panel. **Close Bubble Window** remains bubble-only. Native text/control menus remain intact. Disable **Settings…** and **Close Bubble Window** in the bubble menu while IME marked text is active, and recheck composition when either action runs.
- In the original `bf23ced` implementation, a small menu-bar rescue icon appeared **only** when both windows were hidden or full-window passthrough was on, including after restart and while Settings was open. Normal both-visible, character-only, and interactive bubble-only states used no menu-bar slot. A standalone bubble could show the character through its own menu. This describes historical provenance, not the current working-copy default.
- Alpha pointer polling alone never toggles the rescue icon. **Show and Enable Character / 캐릭터 표시·조작 복구** shows the character and turns off full-window passthrough without changing bubble visibility or alpha passthrough. The rescue menu also offers **Settings…** and **Quit**.

## Current working-copy additions: menu-bar visibility and image

- The default status icon is a template pawprint; a chosen image changes its artwork, not its status item or menu. A native **Menu bar icon / 메뉴 막대 아이콘** setting in the existing three-tab Settings panel offers **Always show / 항상 표시** (`always`) and **Only when recovery is needed / 복구가 필요할 때만 표시** (`recovery_only`). Missing `menu_bar_mode` in `preferences.json` defaults to Always, including existing profiles from the original checkout; no profile recreation is needed. An explicit RecoveryOnly choice persists across restart and unrelated preference saves.
- Always displays the icon while the app runs. RecoveryOnly displays it only when both character and bubble are hidden **or** full-window passthrough is on; interactive standalone bubble alone is not a trigger, nor is alpha pointer passthrough alone. Both modes offer **Settings… / 설정…** and **Quit / 종료**; **Show and Enable Character / 캐릭터 표시·조작 복구** appears only while recovery is needed. Recovery shows the character and clears full-window passthrough, preserving bubble visibility and alpha passthrough. With Always the icon stays; with RecoveryOnly it disappears after the recovery menu closes when no longer needed.
- A successful selection changes only the menu-bar mode; it does not change character/bubble visibility, full-window or alpha passthrough, placement, drafts, or lifecycle. A failed save leaves the prior selected mode and live icon behavior in place. Existing unknown/unrelated preferences are preserved. This is not a new CLI mode command, hotkey, badge, onboarding flow, version, or release.
- **Choose image… / 이미지 선택…** imports a static PNG of up to 4 MiB encoded and 1 million decoded pixels; animation, corruption, truncation, and fully transparent images are rejected. The image retains original colors and aspect-fits within 18 pt with 1×/2× Retina representations. **Restore default icon / 기본 아이콘 복원** returns to the template pawprint. An optional `preferences.json` `menu_bar_icon.asset` references a managed copy under the configuration directory's `menu-bar-icons/`; external originals can be moved or deleted. Back up `menu-bar-icons/` with `preferences.json`, `lifecycle.json`, and `characters/`. Without an icon preference, the default pawprint remains. Import/save failure retains the prior image and settings; a missing/corrupt managed image at restart displays the default with an error but keeps custom metadata. Reset affects only the image, not visibility mode or other settings.
- Source images are opened nonblocking and checked as regular files on the same opened descriptor before any read. This rejects a FIFO with no writer without waiting and still permits symlinks to regular PNGs; the existing byte/pixel limits and managed-asset rules remain unchanged. This is not a universal latency guarantee for network filesystems.

## Current working-copy addition: remove linked worktree

- Right-click or Control-click a local card header to target that card, even if another card is selected; right-clicking the bubble background targets the card selected when the menu opens. The menu labels the exact target. No eligible target means no removal item. Native text/control menus remain intact. **Close Bubble Window** remains hide-only.
- **Remove Worktree… / 워크트리 삭제…** requires a current, live, valid-generation, uniquely coherent linked worktree with positive local metadata; the main repository root is excluded. Older servers with absent/malformed optional metadata remain observable but omit the deletion item; if metadata is available but `worktree.remove` is unsupported, an attempted operation reports that limit without a fallback (unsupported capability alone does not necessarily hide the item). Confirmation defaults to **Cancel** (Return/Escape); **Delete** is the second choice. It shows the repository, exact checkout, workspace, tabs/panes/processes, retained branch, and **IGNORED FILES WILL BE DELETED**: ignored files can be deleted even with `force: false`.
- The captured target is revalidated after confirmation against a fresh snapshot. One operation at a time; IME composition disables deletion and is checked again on action. One dedicated asynchronous `worktree.remove` request carries `{workspace_id, force: false}`; no git/shell fallback, forced deletion, automatic trust, retry, or optimistic removal. Failure and uncertain delivery remain visible in the common bubble after the row disappears, selection changes, or the bubble is hidden and reopened; an uncertain write must not be automatically resent. Normal watcher refresh remains authoritative (about five seconds).
- Review correction: assign the Cancel `NSButtonCell` explicitly as the window default after native alert layout; inability to configure it aborts removal. Compact feedback now leads with a localized outcome, not a long target name. Unknown delivery visibly says to check before retrying. Expanded text, the compact tooltip, and accessibility retain the full checkout and server/offline details. Feedback-only wrapping reserves its measured height before primary dialogue, including the 120 pt minimum body and status ON/OFF; ordinary compact dialogue keeps its two-line policy.
- Native release check, tests, and build passed (332 main tests plus the bounded FIFO child). Behavioral tests cover stale/swap/generation/duplicate metadata, strict acknowledgement, rejection, unknown delivery/no resend, lock-free dispatch, compact result priority, and regular-file/FIFO source boundaries. An isolated newly built AppKit daemon reached UI/control/registration/data readiness; a real disposable Git-worktree exercise using the actual production sender against stock Herdr 0.9.3 removed a clean linked checkout, workspace, panes, and live processes while retaining its branch and unrelated main checkout. Dirty tracked/untracked/submodule and locked worktrees were refused with checkout/workspace/data preserved; main was rejected client-side and by the backend. Correction smoke rejected a no-writer FIFO immediately, preserved regular PNG/symlink colors, and rendered unclipped English/Korean unknown-delivery summaries in native offscreen fields at 96 pt content width, with full target details and status ON/OFF result space. Explicit Cancel default-cell configuration was observed, **not keyboard activation**. **Actual GUI right-click, Return/Enter/Escape (including Delete focused by Tab), tooltip hover, and IME verification remain blocked by a locked macOS session** (`CGSessionScreenIsLocked=1`); permissions alone did not expose windows or enable observable posted clicks. Offscreen rendering and configuration inspection are not proof of those interactions.
- The request has only `workspace_id`, with no expected checkout/generation compare-and-swap: fresh client validation cannot atomically prevent a restart or workspace rebinding after validation. No undo, trash, or branch deletion is provided. This worktree API evidence is independent of client-attach auto-start, which still requires its separate patched host; stock 0.9.3 exercise is not proof of stock 0.9.0 support.

## Independent character and bubble visibility

- Hiding the character leaves an enabled bubble as a movable, tailless panel; showing the character reattaches it. All four visibility combinations remain independent. Save the standalone origin separately and restore it on later detachments.
- Card/reply state, drafts, focus, and active IME survive hide/show during this run. **Drafts do not survive restart.** Hiding a window is not quitting the app or stopping a Herdr session.
- Full-window passthrough applies to **both windows**. Alpha passthrough applies **only to character art**. Standalone placement changes wait for reattachment. Reset resets both positions without changing visibility. **Close Bubble Window** still hides only the bubble.

## Standalone and remote-only watcher startup fix

### Membership is not health

- A valid local `plugin_list` with no `desktop-pet` row on first observation is **not disable**. Keep local snapshots/subscription available and remote polling alive.
- Explicit `enabled: false` detaches. A missing row **after a previously present enabled or disabled row** detaches as observed unlink. Membership history survives rechecks/reconnects within the daemon, **not a new daemon**.
- When **every registered local endpoint is confirmed disabled/unlinked**, quit immediately, even with a healthy remote or `exit_with_herdr` off. Initially missing is not terminal. Other lost-health cases retain the **30-second grace**; this fix does not convert every missing or unhealthy endpoint into an immediate disable.

### Remote-only operation and diagnosis

- The pet runs on the local Mac. Authenticate a saved machine and **explicitly select its enabled profile** under Observation sources. The remote server needs no `desktop-pet` plugin.
- Optionally set `exit_with_herdr` off via the native CLI **before first start when there is no healthy local server**, and enable it after healthy remote polling. This is an explicit user setting choice, not an automatic policy change.
- Turning local observation off excludes display data, **not lifecycle**. It does not bypass confirmed local disable/unlink termination.
- Diagnose an inherited socket separately from the CLI executable/version. There is **no plugin auto-install, new configuration, or SSH transport change** in this fix. For actual commands and host capability requirements, use the [root guide](../../readme.md) and [integration guide](../../integrations/herdr/README.md).

### Regression evidence and method

The original local pending-checkout record describes watcher transport regressions that capture pending-source state **without delaying plugin responses**. A **fresh source** proves worker survival after the disable decision; checking only a retained snapshot would not prove that. Membership history and actual **Enabled restoration** remain checked, so survival alone is not enough to establish correct lifecycle behavior. This is reported evidence for the local implementation, not new verification in this documentation change or evidence of a `main` merge.

Historical native reply probes and test counts belong in the [v0.1.6 evidence appendix](v0.1.6.md#historical-evidence-appendix). They do not establish that these Unreleased source changes have been published.

## Repository release documentation on main

- Reviewed version notes and migration guides now live in the repository; the Wiki is a generated reading view, not a second editable source.
- Future releases validate the exact tag, all app-version manifests, and the reviewed version note before building. The same tagged note supplies the GitHub Release body.
- Wiki publication uses the current default-branch snapshot and live Release/asset records, keeps manually owned pages, and leaves a successful binary Release intact if Wiki publication fails. See [Publishing](publishing.md) for setup and manual repair.
- This documentation and automation change does not publish a binary, assign the next app version, or change the existing v0.1.11 artifacts.

## Upgrade and compatibility boundaries

Read [local pending-checkout upgrade guidance](../migrations/unreleased.md) before building the identified unmerged native implementation; it does not describe native features available from a `main` source build. The underlying authority for baseline installation, controls, lifecycle settings, and pack operations stays in the root guides. Remote cards remain observation-only; submission still requires an eligible selected local session. A prompt acknowledgement is not agent completion. Host capabilities remain independent of the app version, and client-attach auto-start still needs the actual patched host described in the integration guide.

This page assigns neither an app release nor a host release, changes neither character-pack **v5** nor built-in **`default@0`**, and makes no support-lifetime promise. New public releases will use reviewed, tagged version notes as described in [Publishing](publishing.md).

## 한국어: 로컬 미병합 소스 변경과 안전 사항

문서·게시 도구는 `main`에 있지만 아래 네이티브 네 묶음은 로컬 `worktree/rapid-harbor-d2a6`의 `bf23ced1649fd0074aec8736644c8f02aa0c492b`에만 구현된 **미병합 변경**이며 `main`이나 공개 안정판 v0.1.11 바이너리에 없습니다. 메뉴 막대 모드 선택·사용자 지정 아이콘 이미지·워크트리 삭제는 그 과거 커밋이 아닌 **현재 작업본에 별도로 추가된 기능**이며 안정판 바이너리에도 없습니다. `main`에서 `bash scripts/install.sh --source`를 실행해도 이 기능을 얻을 수 없습니다. 명령은 이미 가지고 있는 체크아웃만 빌드하며 이 로컬 브랜치·커밋을 공개 원격에서 가져올 수 있다는 보장은 없습니다. 현재 작업본의 앱 버전은 0.1.11로 유지하고 별도 선택형 베타만 격리 소스 사본에서 버전을 변경합니다. 해당 로컬 구현을 이미 가지고 있다면 [한국어 미병합 업그레이드 안내](../migrations/unreleased.ko.md)를, 기준 앱 설치·실행에는 [루트 안내](../../readme.ko.md)를 따르세요. 네 묶음의 원래 기록과 현재 작업본의 별도 추가 기능을 아래에서 구분합니다.

### 관찰 설정 정리

- **This Mac**과 **Remote machines**를 오른쪽 정렬 네이티브 스위치로 구분하고, **Machines to observe** 아래에 저장된 프로필의 이름·세션·상태를 표시합니다. 줄바꿈한 텍스트 높이를 측정하고 고정 안내 간격·중복 도움말을 제거합니다. 불투명 프로필 ID로 컨트롤을 재사용하여 폴링·이름 변경에도 포커스와 스크롤을 유지하며, 목록이 줄면 스크롤 범위를 제한합니다.
- 최초 목록 조회, 확인된 빈 목록, 미선택, 일시 정지, 확인 중, 관찰 중, 사용 불가, 비활성 상태를 구분합니다. 목록 조회 실패는 이전 목록을 경고와 함께 유지하며 빈 목록으로 취급하지 않습니다. Remote를 꺼도 저장된 선택은 유지하지만 오래된 성공 상태는 숨깁니다.
- 한국어·영어 라벨과 문맥별 접근성 이름을 제공합니다. 등록 도움말·접힌 진단은 읽기 전용입니다. 재연결은 **셸 인용 처리한 명령을 복사하고 네이티브 클립보드 피드백만 표시하며 실행하지 않습니다.**

### 문맥 설정과 조건부 복구 메뉴

- 캐릭터, 대화창 배경, 카드 헤더의 우클릭/Control-click으로 같은 전체 3탭 **Settings…** 패널을 엽니다. 대화창 닫기는 대화창만 숨기며 텍스트·컨트롤의 원래 메뉴는 보존합니다. 대화창 메뉴의 **Settings…**와 **Close Bubble Window**는 IME 조합 중 비활성화하고 실행 시에도 조합 상태를 다시 확인합니다.
- 원래 `bf23ced` 구현의 메뉴 막대 복구 아이콘은 **두 창이 모두 숨겨졌거나 전체 창 클릭 통과가 켜졌을 때만** 나타났으며, 재시작 후와 설정 창이 열려 있을 때도 같았습니다. 두 창 표시, 캐릭터만 표시, 조작 가능한 대화창만 표시하는 상태는 메뉴 막대 공간을 쓰지 않았습니다. 독립 대화창의 메뉴로 캐릭터를 다시 표시할 수 있었습니다. 현재 작업본의 기본값은 아래의 항상 표시입니다.
- 알파 포인터 폴링만으로 아이콘을 켜거나 끄지 않습니다. **캐릭터 표시·조작 복구**는 캐릭터를 표시하고 전체 클릭 통과를 끄지만 대화창 표시 여부와 알파 클릭 통과는 바꾸지 않습니다. 메뉴에는 설정과 종료도 있습니다.

### 현재 작업본 추가 기능: 메뉴 막대 표시 조건·이미지 선택

- 기존 `bf23ced` 기록의 위 조건부 복구 정책은 역사적 근거이며, 지금 작업본의 기본 동작이 아닙니다. 기본 상태 아이콘은 템플릿 발바닥 그림이며 사용자 지정 이미지는 같은 상태 아이템·메뉴의 그림만 바꿉니다. 기존 3탭 설정 패널의 **메뉴 막대 아이콘 / Menu bar icon**에서 **항상 표시 / Always show** (`always`) 또는 **복구가 필요할 때만 표시 / Only when recovery is needed** (`recovery_only`)를 선택합니다. 기존 프로필에서 `preferences.json`의 `menu_bar_mode`가 없어도 항상 표시가 기본이며 프로필 재생성은 필요 없습니다. 명시적으로 선택한 복구 시에만 표시는 재시작과 다른 설정 저장 후에도 유지됩니다.
- 항상 표시는 앱 실행 중 아이콘을 유지합니다. 복구 시에만 표시는 캐릭터·대화창이 **모두 숨겨졌거나** 전체 창 클릭 통과가 켜졌을 때만 아이콘을 표시합니다. 조작 가능한 독립 대화창만 있거나 투명 영역 클릭 통과만 켜진 상태는 조건이 아닙니다. 두 모드 모두 메뉴에 **설정… / Settings…**과 **종료 / Quit**가 있고 복구가 필요할 때만 **캐릭터 표시·조작 복구 / Show and Enable Character**를 추가합니다. 복구는 캐릭터를 표시하고 전체 클릭 통과를 끄되 대화창 표시 여부·투명 영역 클릭 통과는 유지합니다. 항상 표시 모드는 아이콘을 유지하고 복구 시에만 표시는 메뉴를 닫은 뒤 복구 조건이 사라지면 아이콘을 제거합니다.
- 선택 저장이 성공하면 메뉴 막대 모드만 바꾸며 창 표시·전체/투명 영역 클릭 통과·위치·초안·실행 관리는 바꾸지 않습니다. 저장 실패 시 이전 선택과 아이콘 동작을 유지하고 모르는 키·무관한 설정도 보존합니다. 이는 새 CLI 모드 명령·단축키·배지·첫 실행 안내·버전·릴리스가 아닙니다.
- **이미지 선택… / Choose image…**으로 정적 PNG(인코딩 최대 4 MiB, 디코딩 후 최대 100만 픽셀)를 가져옵니다. 애니메이션·손상·잘림·완전 투명 이미지는 거부합니다. 원래 색을 유지하고 18 pt 안에 비율에 맞춰 배치하며 1×/2× Retina 표현을 사용합니다. **기본 아이콘 복원 / Restore default icon**으로 템플릿 발바닥 그림으로 돌아갑니다. 선택적인 `preferences.json`의 `menu_bar_icon.asset`은 설정 디렉터리의 `menu-bar-icons/`에 저장한 관리 사본을 가리켜 외부 원본을 옮기거나 삭제해도 유지됩니다. `preferences.json`, `lifecycle.json`, `characters/`와 함께 `menu-bar-icons/`를 백업하세요. 아이콘 설정이 없으면 기본 그림을 쓰며, 가져오기·저장 실패 시 이전 이미지와 설정을 유지합니다. 재시작할 때 관리 이미지가 없거나 손상되면 오류와 함께 기본 아이콘을 표시하되 사용자 지정 메타데이터는 보존합니다. 기본 아이콘 복원은 이미지에만 영향을 주며 표시 모드·다른 설정은 바꾸지 않습니다.
- 원본 이미지는 nonblocking으로 열고 **같은 열린 descriptor**에서 일반 파일인지 확인한 뒤 읽습니다. writer 없는 FIFO를 기다리지 않고 거부하며 일반 PNG를 가리키는 symlink는 허용합니다. 기존 바이트·픽셀 제한과 관리 사본 규칙은 유지합니다. 모든 네트워크 파일시스템의 지연을 보장하는 변경은 아닙니다.

### 현재 작업본 추가 기능: 연결된 워크트리 삭제

- 로컬 카드 헤더의 우클릭/Control-click은 선택된 카드와 관계없이 클릭한 카드를 대상으로 하고, 대화창 배경은 메뉴를 열 때 선택된 카드를 대상으로 합니다. 메뉴는 대상을 명시하며 적격 대상이 없으면 삭제 항목이 없습니다. 텍스트·컨트롤 네이티브 메뉴는 유지하고 **대화창 닫기**는 여전히 숨기기만 합니다.
- **워크트리 삭제… / Remove Worktree…**는 현재 로컬의 실행 중인 유효한 세대에서 중복 없이 일치하는 연결 워크트리 메타데이터가 있어야 합니다. 메인 저장소 루트는 제외합니다. 선택적 메타데이터가 없거나 잘못되면 관찰은 유지하지만 삭제 항목은 표시하지 않습니다. 메타데이터가 있으나 서버가 `worktree.remove`를 지원하지 않으면 실행 시 우회 없이 미지원 한계를 알리며, 미지원만으로 항목이 반드시 숨겨지는 것은 아닙니다. 확인 창은 **취소**가 첫 번째·기본 선택(Return/Escape), **삭제**가 두 번째이며 저장소·정확한 체크아웃·워크스페이스·탭/패널/프로세스·브랜치 유지와 **무시된 파일도 삭제됩니다**를 표시합니다. `force: false`여도 무시된 파일은 삭제될 수 있습니다.
- 대상은 고정한 뒤 확인 후 새 스냅샷에서 재검증합니다. 한 번에 하나만 실행하며 IME 조합 중 삭제를 비활성화하고 실행 시 다시 확인합니다. 전용 비동기 `worktree.remove` 요청 하나에 `{workspace_id, force: false}`를 보내며 git/셸 우회·강제 삭제·자동 신뢰·재시도·낙관적 제거가 없습니다. 행 삭제·선택 변경·창 숨김 후 재표시에도 공통 대화창 피드백을 유지합니다. 쓰기 시도 후 전달 여부가 불분명하면 명시하고 자동 재전송하지 않습니다. 일반 watcher의 약 5초 갱신이 최종 관찰 기준입니다.
- 리뷰 수정: 네이티브 확인 창의 layout 뒤 취소 `NSButtonCell`을 창 기본 버튼으로 명시하며, 안전하게 설정할 수 없으면 삭제를 중단합니다. 작은 보기에서는 긴 대상명보다 지역화된 결과를 먼저 표시하고 전달 불명 시 재시도 전 확인을 안내합니다. 펼친 본문·작은 보기 tooltip·접근성에는 전체 체크아웃과 서버·offline 상세를 유지합니다. 피드백이 있을 때만 줄 수 제한을 풀고 실제 높이를 먼저 확보하여 최소 120 pt 본문과 상태 ON/OFF에서도 주 대화보다 결과를 우선합니다. 일반 작은 보기 대화의 두 줄 정책은 유지합니다.
- 네이티브 release 검사·테스트·빌드가 통과했습니다(주 테스트 332개와 제한시간이 있는 FIFO child). 동작 테스트는 오래된/교체된 대상·세대·중복·엄격한 응답·거부·전달 불명/재전송 방지·잠금 없는 전송·작은 보기 결과 우선 배치·일반 파일/FIFO 입력 경계를 다룹니다. 새 AppKit 데몬의 격리 실행에서 UI/control/registration/data 준비를 확인했고, 실제 production sender와 순정 Herdr 0.9.3의 일회용 Git 워크트리 실험에서는 깨끗한 연결 체크아웃·워크스페이스·패널·실행 프로세스가 제거되고 브랜치·무관한 메인 체크아웃은 유지되었습니다. 변경된 tracked/untracked/submodule 및 잠긴 워크트리는 거부되어 데이터가 유지됐고 메인 루트는 클라이언트·백엔드에서 거부됐습니다. 수정 smoke에서는 writer 없는 FIFO의 즉시 거부·일반 PNG/symlink 색 보존을 확인했고, 실제 네이티브 offscreen 필드의 96 pt 폭에서 한·영 전달 불명 안내가 잘리지 않으며 전체 대상 상세와 상태 ON/OFF 결과 공간이 유지됨을 확인했습니다. 취소 기본 cell 설정은 관찰했지만 **키 입력에 따른 실행은 검증하지 못했습니다**. **macOS 세션 잠금**(`CGSessionScreenIsLocked=1`)으로 실제 GUI 우클릭·Return/Enter/Escape(Tab으로 삭제에 포커스를 둔 경우 포함)·tooltip hover·IME 검증은 여전히 막혀 있습니다. offscreen 렌더링과 설정 확인을 해당 조작의 검증 완료로 해석하지 마세요.
- API는 `workspace_id`만 받고 예상 체크아웃/세대의 원자적 대조(CAS)가 없습니다. 클라이언트 재검증 뒤 재시작·워크스페이스 재연결 경합까지 원자적으로 막는다고 보장하지 않습니다. 되돌리기·휴지통·브랜치 삭제는 없으며 이 API의 호환성은 별도 패치가 필요한 client-attach 자동 시작과 독립적입니다. 순정 0.9.3 실험을 0.9.0 지원 증거로 해석하지 마세요.

### 캐릭터·대화창 표시 독립

- 캐릭터를 숨겨도 활성 대화창은 꼬리 없는 이동 가능한 패널로 남고, 캐릭터를 표시하면 다시 붙습니다. 네 가지 표시 조합은 독립적입니다. 독립 대화창 위치는 별도로 저장하고 나중에 다시 분리할 때 복원합니다.
- 실행 중 숨김/표시 전환에도 카드·답장 상태, 초안, 포커스, 활성 IME를 유지합니다. **초안은 재시작 후 복원하지 않습니다.** 숨김은 앱이나 Herdr 세션 종료가 아닙니다.
- 전체 클릭 통과는 **두 창 모두**, 알파 클릭 통과는 **캐릭터 그림에만** 적용합니다. 독립 상태의 대화창 배치 설정 변경은 다시 붙을 때까지 기다립니다. 위치 초기화는 두 위치만 초기화하며 표시 여부는 바꾸지 않습니다. 대화창 닫기는 여전히 대화창만 숨깁니다.

### 독립 실행·원격 전용 관찰 시작 수정

- 최초 로컬 `plugin_list`에 `desktop-pet` 행이 없어도 정상 목록이라면 비활성화가 아닙니다. 로컬 스냅샷·구독과 원격 폴링을 유지합니다. 명시적 `enabled: false`는 분리하며, 이전에 활성/비활성 행이 있었는데 이후 사라지면 관찰된 연결 해제로 분리합니다. 이 이력은 같은 데몬의 재확인·재연결 동안 유지하지만 새 데몬에는 이어지지 않습니다.
- **등록된 모든 로컬 엔드포인트가 비활성화/연결 해제로 확인되면 즉시 종료**합니다. 정상 원격이 있거나 `exit_with_herdr`가 꺼져 있어도 같습니다. 처음부터 행이 없는 것은 종료 조건이 아니며 다른 정상 연결 상실은 기존 **30초 유예**를 유지합니다.
- 원격 전용이어도 펫은 로컬 Mac에서 실행합니다. 저장된 머신을 인증한 뒤 관찰 소스에서 **활성 프로필을 직접 선택**하세요. 원격 서버에는 `desktop-pet` 플러그인이 필요 없습니다. 정상 로컬 서버 없이 처음 시작하려면 선택적으로 네이티브 CLI에서 `exit_with_herdr`를 미리 끄고, 정상 원격 폴링을 확인한 뒤 켤 수 있습니다. 자동 정책 변경이 아닙니다.
- 로컬 관찰을 끄는 것은 표시 데이터만 제외하며 **수명주기 정책은 끄지 않습니다**. 상속된 소켓 문제와 CLI 실행 파일/버전 문제를 따로 진단합니다. 플러그인 자동 설치, 새 설정, SSH 전송 변경은 없습니다. 원격 카드는 여전히 읽기 전용이고, 로컬 프롬프트 접수 확인은 작업 완료가 아닙니다.
- 원래 회귀 기록은 플러그인 응답을 지연하지 않고 pending-source 상태를 수집하며, 비활성화 판단 뒤 **새 소스**로 워커 생존을 입증합니다. 멤버십 이력과 실제 Enabled 복원도 확인해야 합니다. 과거 네이티브 프로브·테스트 수는 당시 보고된 근거일 뿐 이 문서 작업의 새 검증, 실제 키보드 IME·OS 포커스 보장, 공개 배포 증거가 아닙니다.

### main의 저장소 버전 문서와 발행 자동화

문서·게시 도구 변경은 위의 로컬 미병합 네이티브 네 묶음과 별개로 `main`에 반영됩니다. 검토한 버전별 기록·마이그레이션 문서는 저장소가 원본이며 Wiki는 생성된 읽기 화면입니다. 앞으로 릴리스는 태그·앱 버전 매니페스트·검토한 해당 버전 문서를 빌드 전에 확인하고 같은 태그 문서로 GitHub Release 본문을 생성합니다. Wiki는 기본 브랜치의 고정된 커밋과 실제 Release·업로드 완료 자산으로 생성하며 수동 작성 페이지를 유지합니다. Wiki 발행 실패는 이미 성공한 바이너리 Release를 취소하지 않습니다. [운영·복구 절차](publishing.md)를 따르세요. 이 문서·자동화 변경은 네이티브 기능 병합, 새 바이너리 발행이나 다음 버전 지정, 기존 v0.1.11 배포 파일 변경이 아닙니다.
