# Herdr Desktop Pet Unreleased

[Versions](README.md) · [0.1 line](0.1.md) · [Versioning and compatibility](policy.md) · [Upgrade guidance](../migrations/unreleased.md) · [한국어 업그레이드 안내](../migrations/unreleased.ko.md)

**Distinct source authorities are documented here.** The repository documentation and publishing tools are on `main`. The four native feature groups below are implemented only in local `worktree/rapid-harbor-d2a6` at `bf23ced1649fd0074aec8736644c8f02aa0c492b`; they are **unmerged and absent from `main` and the public v0.1.11 binary**. The selectable menu-bar mode described separately below is a **current working-copy addition**, not a feature attributed to that historical commit or to the public binary. The app version remains **0.1.11**; no next release version, release date, or native-preview download is assigned. A source commit or Wiki publication does not publish a binary release.

| Source | What it provides |
| --- | --- |
| Public v0.1.11 binary | Published baseline; none of the four pending native groups |
| `main` source | Baseline native implementation plus repository documentation and publishing tools; none of the four pending native groups |
| Local `worktree/rapid-harbor-d2a6` at `bf23ced1649fd0074aec8736644c8f02aa0c492b` | The four implemented but unmerged native groups; guidance only for users who already possess this checkout |
| Current local working copy of `worktree/rapid-harbor-d2a6` | The four pending groups plus the separately added selectable menu-bar mode; neither is available from `main` or the public binary |

`bash scripts/install.sh --source` builds the checkout you already have; running it on `main` does **not** install these native previews. The local branch name and commit identify the implementation, not a guaranteed public remote branch, fetchable commit, or download. Follow the bilingual root [English](../../readme.md) / [Korean](../../readme.ko.md) guides for baseline installation and runtime commands. Use the [pending-checkout upgrade guidance](../migrations/unreleased.md) only for that local implementation.

All four pending native groups from the original pending-source record are retained below, separately from the main-branch documentation/tools change. The four historically identified group sections describe the **local checkout only**, not current `main` behavior or shipped binary features. The historical conditional-rescue policy in the second group is superseded **only in the current working copy** by the separately labeled menu-bar mode section after it.

## Compact observation settings

- Separate **This Mac** and **Remote machines** with right-aligned native switches. Nest saved profiles under **Machines to observe**, showing name, session, and status.
- Measure wrapped text for card, row, and scroll heights; remove fixed notice gaps and duplicate help. Reuse controls by opaque profile ID so polling and renaming preserve focus and scroll. Clamp scrolling when the list shrinks.
- Distinguish **initial catalog lookup**, **confirmed empty**, **no selection**, **paused**, **checking**, **observing**, **unavailable**, and **disabled**. Catalog failure retains the previous profile list with an explicit warning; it is not a confirmed empty result. Turning Remote off keeps saved selections and masks stale successful statuses.
- Localize labels and contextual accessibility names in Korean and English. Registration help and folded diagnostics are read-only. Reconnect copies a **shell-quoted command** with native clipboard feedback; it **never executes** that command.

## Contextual settings and conditional rescue

- Right-click or Control-click the character, bubble background, or card header for **Settings…** in the same full three-tab panel. **Close Bubble Window** remains bubble-only. Native text/control menus remain intact. Disable **Settings…** and **Close Bubble Window** in the bubble menu while IME marked text is active, and recheck composition when either action runs.
- In the original `bf23ced` implementation, a small menu-bar rescue icon appeared **only** when both windows were hidden or full-window passthrough was on, including after restart and while Settings was open. Normal both-visible, character-only, and interactive bubble-only states used no menu-bar slot. A standalone bubble could show the character through its own menu. This describes historical provenance, not the current working-copy default.
- Alpha pointer polling alone never toggles the rescue icon. **Show and Enable Character / 캐릭터 표시·조작 복구** shows the character and turns off full-window passthrough without changing bubble visibility or alpha passthrough. The rescue menu also offers **Settings…** and **Quit**.

## Current working-copy addition: selectable menu-bar icon

- The single pawprint status icon identifies **Herdr Desktop Pet**. A new native **Menu bar icon / 메뉴 막대 아이콘** setting in the existing three-tab Settings panel offers **Always show / 항상 표시** (`always`) and **Only when recovery is needed / 복구가 필요할 때만 표시** (`recovery_only`). Missing `menu_bar_mode` in `preferences.json` defaults to Always, including existing profiles from the original checkout; no profile recreation is needed. An explicit RecoveryOnly choice persists across restart and unrelated preference saves.
- Always displays the icon while the app runs. RecoveryOnly displays it only when both character and bubble are hidden **or** full-window passthrough is on; interactive standalone bubble alone is not a trigger, nor is alpha pointer passthrough alone. Both modes offer **Settings… / 설정…** and **Quit / 종료**; **Show and Enable Character / 캐릭터 표시·조작 복구** appears only while recovery is needed. Recovery shows the character and clears full-window passthrough, preserving bubble visibility and alpha passthrough. With Always the icon stays; with RecoveryOnly it disappears after the recovery menu closes when no longer needed.
- A successful selection changes only the menu-bar mode; it does not change character/bubble visibility, full-window or alpha passthrough, placement, drafts, or lifecycle. A failed save leaves the prior selected mode and live icon behavior in place. Existing unknown/unrelated preferences are preserved. This is not a new CLI mode command, hotkey, badge, onboarding flow, version, or release.

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

문서·게시 도구는 `main`에 있지만 아래 네이티브 네 묶음은 로컬 `worktree/rapid-harbor-d2a6`의 `bf23ced1649fd0074aec8736644c8f02aa0c492b`에만 구현된 **미병합 변경**이며 `main`이나 공개 v0.1.11 바이너리에 없습니다. 이 문서의 메뉴 막대 모드 선택은 그 과거 커밋이 아닌 **현재 작업본에 별도로 추가된 기능**이며 공개 바이너리에도 없습니다. `main`에서 `bash scripts/install.sh --source`를 실행해도 이 기능을 얻을 수 없습니다. 명령은 이미 가지고 있는 체크아웃만 빌드하며 이 로컬 브랜치·커밋을 공개 원격에서 가져올 수 있다는 보장은 없습니다. 앱 버전은 0.1.11로 유지하며 다음 버전·배포 날짜·다운로드는 지정하지 않습니다. 해당 로컬 구현을 이미 가지고 있다면 [한국어 미병합 업그레이드 안내](../migrations/unreleased.ko.md)를, 기준 앱 설치·실행에는 [루트 안내](../../readme.ko.md)를 따르세요. 네 묶음의 원래 기록과 현재 작업본의 별도 추가 기능을 아래에서 구분합니다.

### 관찰 설정 정리

- **This Mac**과 **Remote machines**를 오른쪽 정렬 네이티브 스위치로 구분하고, **Machines to observe** 아래에 저장된 프로필의 이름·세션·상태를 표시합니다. 줄바꿈한 텍스트 높이를 측정하고 고정 안내 간격·중복 도움말을 제거합니다. 불투명 프로필 ID로 컨트롤을 재사용하여 폴링·이름 변경에도 포커스와 스크롤을 유지하며, 목록이 줄면 스크롤 범위를 제한합니다.
- 최초 목록 조회, 확인된 빈 목록, 미선택, 일시 정지, 확인 중, 관찰 중, 사용 불가, 비활성 상태를 구분합니다. 목록 조회 실패는 이전 목록을 경고와 함께 유지하며 빈 목록으로 취급하지 않습니다. Remote를 꺼도 저장된 선택은 유지하지만 오래된 성공 상태는 숨깁니다.
- 한국어·영어 라벨과 문맥별 접근성 이름을 제공합니다. 등록 도움말·접힌 진단은 읽기 전용입니다. 재연결은 **셸 인용 처리한 명령을 복사하고 네이티브 클립보드 피드백만 표시하며 실행하지 않습니다.**

### 문맥 설정과 조건부 복구 메뉴

- 캐릭터, 대화창 배경, 카드 헤더의 우클릭/Control-click으로 같은 전체 3탭 **Settings…** 패널을 엽니다. 대화창 닫기는 대화창만 숨기며 텍스트·컨트롤의 원래 메뉴는 보존합니다. 대화창 메뉴의 **Settings…**와 **Close Bubble Window**는 IME 조합 중 비활성화하고 실행 시에도 조합 상태를 다시 확인합니다.
- 원래 `bf23ced` 구현의 메뉴 막대 복구 아이콘은 **두 창이 모두 숨겨졌거나 전체 창 클릭 통과가 켜졌을 때만** 나타났으며, 재시작 후와 설정 창이 열려 있을 때도 같았습니다. 두 창 표시, 캐릭터만 표시, 조작 가능한 대화창만 표시하는 상태는 메뉴 막대 공간을 쓰지 않았습니다. 독립 대화창의 메뉴로 캐릭터를 다시 표시할 수 있었습니다. 현재 작업본의 기본값은 아래의 항상 표시입니다.
- 알파 포인터 폴링만으로 아이콘을 켜거나 끄지 않습니다. **캐릭터 표시·조작 복구**는 캐릭터를 표시하고 전체 클릭 통과를 끄지만 대화창 표시 여부와 알파 클릭 통과는 바꾸지 않습니다. 메뉴에는 설정과 종료도 있습니다.

### 현재 작업본 추가 기능: 메뉴 막대 아이콘 선택

- 기존 `bf23ced` 기록의 위 조건부 복구 정책은 역사적 근거이며, 지금 작업본의 기본 동작이 아닙니다. **Herdr Desktop Pet** 발바닥 상태 아이콘 하나를 사용합니다. 기존 3탭 설정 패널의 **메뉴 막대 아이콘 / Menu bar icon**에서 **항상 표시 / Always show** (`always`) 또는 **복구가 필요할 때만 표시 / Only when recovery is needed** (`recovery_only`)를 선택합니다. 기존 프로필에서 `preferences.json`의 `menu_bar_mode`가 없어도 항상 표시가 기본이며 프로필 재생성은 필요 없습니다. 명시적으로 선택한 복구 시에만 표시는 재시작과 다른 설정 저장 후에도 유지됩니다.
- 항상 표시는 앱 실행 중 아이콘을 유지합니다. 복구 시에만 표시는 캐릭터·대화창이 **모두 숨겨졌거나** 전체 창 클릭 통과가 켜졌을 때만 아이콘을 표시합니다. 조작 가능한 독립 대화창만 있거나 투명 영역 클릭 통과만 켜진 상태는 조건이 아닙니다. 두 모드 모두 메뉴에 **설정… / Settings…**과 **종료 / Quit**가 있고 복구가 필요할 때만 **캐릭터 표시·조작 복구 / Show and Enable Character**를 추가합니다. 복구는 캐릭터를 표시하고 전체 클릭 통과를 끄되 대화창 표시 여부·투명 영역 클릭 통과는 유지합니다. 항상 표시 모드는 아이콘을 유지하고 복구 시에만 표시는 메뉴를 닫은 뒤 복구 조건이 사라지면 아이콘을 제거합니다.
- 선택 저장이 성공하면 메뉴 막대 모드만 바꾸며 창 표시·전체/투명 영역 클릭 통과·위치·초안·실행 관리는 바꾸지 않습니다. 저장 실패 시 이전 선택과 아이콘 동작을 유지하고 모르는 키·무관한 설정도 보존합니다. 이는 새 CLI 모드 명령·단축키·배지·첫 실행 안내·버전·릴리스가 아닙니다.

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
