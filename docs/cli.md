# Herdr Desktop Pet CLI contract

This is the **v0.3.3 stable-source/default-prebuilt-target CLI contract**. Source manifests and documentation alone do not prove that a public archive, installer or formula is available. Earlier public v0.2.0/v0.2.1 and historical beta binaries do not gain these commands by changing documentation. Build this checkout with `--source`, or confirm the exact non-draft v0.3.3 Release, complete assets and updated installer/formula before using a prebuilt; check the CLI version **and running daemon executable**. Examples assume `herdr-desktop-pet` resolves to that build and an active desktop daemon unless noted; JSON examples show fields and types, not IDs to paste. No command imports a menu image; there is no GUI draft editing or testing API and no physical-input certification.

## English

### Connection, identity, and operations

`presentation`, `preferences`, `sessions`, `dialogue`, and `worktree` use the running daemon's private same-UID Unix control socket at `<state-dir>/control.sock`. Use the daemon's matching `--config-dir PATH` and `--state-dir PATH`; **`--socket PATH` selects the Herdr backend, not this control socket**. There is no HTTP endpoint, daemon-side reading of CLI `--file` inputs, implicit daemon startup, shell fallback, or offline mutation for these families. Existing `ensure`, `start`, `stop`, `restart`, `settings get`, `settings set auto_start on|off`, and `settings set exit_with_herdr on|off` manage **lifecycle settings**, not `preferences set`; `status` reports lifecycle/daemon state. Existing imperative show/hide/bubble/scale controls remain separate from absolute `presentation set`.

An `instance_id` is a daemon-lifetime nonce: obtain it from `presentation get` or other read results, and do not reuse it after restart. A session is the complete four-part key `(instance_id, source_id, generation, terminal_id)`; source/terminal alone or a GUI-selected first row is not an identity. Mutations generate an `operation_id` if omitted; supply a unique `--operation-id ID` **before submission** when recovery after lost ACK matters. Never blindly resend a mutation after a disconnect or timeout. Query its matching family status instead. `presentation status OPID --instance ID` uses a positional ID; `preferences|sessions|dialogue|worktree status --instance ID --operation-id OPID` are separate family-scoped lookups (a different family's operation is rejected). `pack status OPID` is separate again. An unknown/expired operation or a restarted daemon does **not** prove the original request never ran.

Default mutation wait is 15 seconds. `--no-wait` acknowledges accepted/pending work, not persistence or native application; an **already terminal failure** (including pack `failed`, `canceled`, `durability_unknown`, `committed_pending_apply`, or `unknown`) still exits nonzero, while a completed pack operation succeeds. `--wait SECONDS` is finite (presentation accepts 0–86400; the other domain families require >0–86400). Domain mutation/read and pack polling start their absolute wait deadline after their submission ACK; the partial-palette snapshot read likewise starts its own deadline after its read ACK, and must finish before any patch mutation is submitted. Each deadline covers sleeps and complete status RPCs, including trickled responses; a late terminal reply is not success. Domain **mutation** wait expiry prints `{"ok":false,"deadline_exceeded":true,"operation":…}` with the last known pending operation and a status-query error; domain **read** expiry instead returns a pending/status-query error without that JSON field. Presentation has `deadline_exceeded` in its mutation/status envelope. Early transport/protocol failure reports uncertainty instead of pretending the overall deadline elapsed. Expiry does not cancel, roll back, or resend. Read operation `state`, `committed`, `native_applied`, and result independently; a successful exit/ACK is not proof of agent answer, filesystem deletion, or AppKit application.

Domain mutations print `{"ok":boolean,"operation":{"instance_id":string,"operation_id":string,"kind":string,"state":string,"committed":boolean,"native_applied":boolean,"result":object|null,"error_code":string|null,"error":string|null}}`. `state` can be `accepted`, `pending`, `applied`, `agent_prompted`, `failed`, `unknown_delivery`, `superseded`, or `shutdown`; `agent_prompted` means the prompt delivery was acknowledged, **not** that the model completed it. Domain status uses the same wrapper; failed terminal states return an error. A rejected request may not have an operation result. No-wait `ok:true` can describe a still-pending operation. Read-only session/dialogue/worktree commands below print their result object directly, not this wrapper. Values may be null until observed.

### App update status and recovery

```text
herdr-desktop-pet update-capabilities
herdr-desktop-pet update-status [OPID] --state-dir PATH
```

These read-only commands work without a running daemon. This checkout's `update-capabilities` prints `{"protocol":2}`; updater IDs are exactly 32 lowercase hexadecimal characters, distinct from domain-family operation IDs. With no ID, `update-status` selects the active reservation before the last outcome. A protocol-2 plan and journal both require `version:2`; the journal requires `execution_fence` (`no_spawn`, `manager_intent`, or `manager_exited`). `no_spawn` means no manager was launched only when corroborated by absent manager artifacts; `manager_intent` is durable before spawning and a missing PID/exit still means **unknown**, not safe-to-retry; `manager_exited` requires exit and drained output evidence. The phase, fence, `installed`, and `applied` are distinct facts. Offline `update-status` reports durable history, plan/error, and exact retained-helper argv when available; it does not inspect current processes, run the helper, or verify its signature. A `Failed` outcome can remain in history even after safe reconciliation; a failure exit code does not imply the reservation is still held.

Verify the private retained helper's code signature before using its `status --state-dir PATH --operation-id OPID` to inspect current helper/manager/candidate evidence. Its explicit `recover --state-dir PATH --operation-id OPID [--start]` holds a per-operation lease: recovery is rejected while the original helper owns that lease. It reconciles without reinstalling. With proved `no_spawn`, absent manager artifacts and the exact original ready PID/instance/image/source/profile, recovery can record `Failed` with `installed:false`, `applied:false` and release the reservation; if the unchanged original is stopped it can settle without starting it. A restored, *fresh* original instance is checked for readiness before stopped-original logic and remains `Failed`, not an upgrade. Only explicit `--start` may start a verified installed candidate or restore a verified original; an installed candidate need not have been applied. Partial marker release is recovered idempotently; an incomplete/ambiguous release, active or unproven manager/group/output, missing evidence, different profile, conflicting live installation user, or newer user stop keeps safety ahead of startup. `Unknown` retains the reservation and never replays installation. Do not delete markers or blindly retry; after a safely settled failure, a **new** check and explicit user consent are a separate operation, not replay of the old one. `run` is the application's signed-helper handoff, not a general-purpose install CLI.

A retained operation's `run` rejects an existing journal before changing any execution fence or lifecycle evidence. Use `recover`, never rerun installation. A verified externally installed candidate can remain `Failed` with `installed:true`, `applied:false` and a retained reservation until explicit `recover --start` or a superseding user Stop; an unread accepted Stop reply is not evidence that the server's reply write failed.

Protocol-1 plans, journals and reservation markers are blocked and preserved, not migrated, defaulted, aliased or silently deleted. The source/ref and original injected Herdr host plugin config identify the managed checkout separately from its plugin subdirectory, socket and override profile; a Herdr checkout root, local root, or Homebrew formula's physical Cellar root (not the whole Brew prefix) defines installation conflict scope. The selected profile and external assets are retained rather than falling back to defaults. A user Stop always overrides updater restart.

### Presentation

```text
herdr-desktop-pet presentation get
herdr-desktop-pet presentation set (--visible on|off | --passthrough on|off | --alpha-passthrough on|off | --bubble-visible on|off | --bubble-placement above|below|left|right|auto | --scale NUMBER) [more distinct fields] [--expected-revision N] [--operation-id ID] [--wait SECONDS | --no-wait]
herdr-desktop-pet presentation reset [--expected-revision N] [--operation-id ID] [--wait SECONDS | --no-wait]
herdr-desktop-pet presentation status OPID --instance ID
```

`set` requires at least one absolute field; `--scale` is a finite absolute renderer scale (not `bigger`/`smaller`). `reset` resets pet and standalone bubble positions, preserving visibility. `get` prints `{"ok":true,"instance_id":"…","snapshot":{"instance_id":"…","revision":N,"desired":{…},"effective":{…}|null,"persisted":{…}|null,"pet_window_visible":boolean|null,"bubble_window_visible":boolean|null,"pet_window_frame":{x,y,width,height}|null,"bubble_window_frame":{x,y,width,height}|null,"pending_reasons":[…]}}`. Native window state may lag desired state. Mutation/status prints `{"ok":boolean,"instance_id":"…","operation":{"instance_id":"…","operation_id":"…","state":"accepted|pending|applied|superseded|persist_failed|rejected|shutdown|unknown","revision":N|null,"target":{…}|null,"native_applied":boolean,"persisted":boolean,"error":string|null},"deadline_exceeded":boolean}`; an early rejection/uncertain transmission instead prints `state:"rejected"`/`"unknown"`, operation ID, instance ID, and error. `--expected-revision` is a compare-and-set guard. Presentation applies runtime-first: native application and the exact target's successful persistence are **separate**; `persist_failed` is not hidden by a native change, and unrelated saves cannot certify this operation. An uncommitted candidate must not become a ghost persisted value.
Within one accepted presentation batch, explicitly requested fields among visibility, passthrough, alpha passthrough, bubble visibility, placement, and scale are saved from the final normalized scene, even if a later same-target request is a no-op. A rejected candidate cannot leak into a later bubble-only save; a later different full target retains normal supersession behavior.

### Preferences

```text
herdr-desktop-pet preferences get
herdr-desktop-pet preferences set [--language system|ko|en] [--theme warm_ivory|dusty_rose|moonlit_ink|custom] [--surface #RRGGBB] [--text #RRGGBB] [--muted #RRGGBB] [--border #RRGGBB] [--accent #RRGGBB] [--status-indicators on|off] [--menu-bar always|recovery_only] [--observation-local on|off] [--observation-remote on|off] [--machine ID ... | --clear-machines] [--expected-revision N] [--operation-id ID] [--wait SECONDS | --no-wait]
herdr-desktop-pet preferences status --instance ID --operation-id OPID
```

`set` requires at least one setting. Repeated `--machine` selects listed catalog IDs; `--clear-machines` conflicts with `--machine`. Unknown/disabled machine IDs fail with typed `error_code` (for example `unknown_machine`); inspect the catalog first. A partial color palette reads the saved palette, merges only supplied colors and binds the resulting write to its saved revision; a concurrent change fails instead of silently overwriting colors. Optional `--expected-revision` rejects concurrent settings changes (`revision_conflict`). `get` prints a domain wrapper whose `operation.result` contains `revision`, `desired`, `persisted`, `effective`, `pending_reasons`, `status_item_visible`, `effective_locale`, and `observation_catalog` (`initialized`, `machines` with IDs, labels, enabled state and status). Preference snapshots include `language`, `bubble_appearance`, `show_status_indicators`, `menu_bar_mode`, `observation_local`, `observation_remote`, and `observation_machines`. Preferences save their candidate **before** applying runtime changes; failed save leaves prior saved/runtime settings unchanged, not a ghost candidate. `committed:true` means saved, while `native_applied`/pending reasons describe the later UI state. These are not lifecycle `settings` flags.
CLI `--language` accepts **exactly** `system`, `ko`, or `en` (including `--language=VALUE`), rejecting typos, case changes, empty or padded values before profile/socket/storage access; loading an older disk preference with an unknown language still uses its tolerant fallback. CLI `--machine` replaces the **whole list** and requires every ID to be currently enabled. The GUI's trusted selection delta can retain or remove an already saved missing/disabled ID and add currently enabled IDs only; it does not loosen CLI validation. Quiet, visible color settings refresh only conflict/marked-text eligibility and labels, not field drafts, focus, selection, undo, or swatches; closing the panel stops that refresh.

### Sessions and agent prompt

```text
herdr-desktop-pet sessions list --instance ID [--filter all|idle|working|waiting|completed|unknown|offline] [--limit 1..128]
herdr-desktop-pet sessions show --instance ID --source N --generation N --terminal ID
herdr-desktop-pet sessions prompt --instance ID --source N --generation N --terminal ID (--text TEXT | --file PATH | --stdin) [--operation-id ID] [--wait SECONDS | --no-wait]
herdr-desktop-pet sessions status --instance ID --operation-id OPID
```

`list` returns one complete collected JSON object `{instance_id,revision,filter,total,matched,status_summary,rows,next_cursor:null}`. `--limit` (default 32, range 1–128) is **page size, not total row cap**: the CLI fetches all pages, including results beyond 128 rows, checks instance/revision/filter/count consistency and fails rather than printing an inconsistent partial collection. Each row and `show` result contains `key:{instance_id,source_id,generation,terminal_id}`, `source_label`, `is_local`, `pane_id`, `availability`, `status`, `outcome`, `display_status`, and displayable `metadata` (title, agent, workspace/tab IDs and labels); they exclude raw source socket paths, working-directory paths, and worktree checkout/root paths. `show` requires the exact key and can read a retained offline row; that does not mean it can be prompted. A prompt targets only an eligible live local source, never a remote/read-only/stale/offline source. `--text`, `--file`, `--stdin` are mutually exclusive and file/stdin are read by the **client**, not by the daemon. Input is UTF-8; the admission bound is a 512 KiB raw prompt budget (the CLI reserves 4096 bytes, accepting at most 520192 UTF-8 text bytes), with a dedicated larger prompt request envelope; ordinary domain requests/replies have separate smaller control-frame bounds (16 KiB request, 64 KiB result; prompt envelope about 516 KiB and larger reply about 68 KiB). A prompt operation reaching `agent_prompted` only proves upstream prompt ACK, not exact model completion; inspect session status/outcome separately.
Each daemon `sessions list` page has a separate **512 KiB bound on the entire encoded JSON reply including envelope, escaping, cursor and trailing newline**, rather than the ordinary 64 KiB domain result bound. It returns the largest fitting row prefix up to `--limit`; if even one row and its required cursor cannot fit, it returns a bounded error rather than a truncated row or empty continuation. The CLI's collected output can exceed 512 KiB across pages.

Read-only example (copy the returned instance and exact row key manually):

```sh
herdr-desktop-pet presentation get
herdr-desktop-pet sessions list --instance '<instance-from-get>' --filter all --limit 32
herdr-desktop-pet sessions show --instance '<row.key.instance_id>' --source '<row.key.source_id>' --generation '<row.key.generation>' --terminal '<row.key.terminal_id>'
```

### Dialogue

```text
herdr-desktop-pet dialogue list
herdr-desktop-pet dialogue get --target JSON --locale ko|en --slot KEY
herdr-desktop-pet dialogue set --target JSON --locale ko|en --slot KEY (--text TEXT | --file PATH | --stdin) [--baseline JSON] [--operation-id ID] [--wait SECONDS | --no-wait]
herdr-desktop-pet dialogue reset-entry --target JSON --locale ko|en --slot KEY [--baseline JSON] [--operation-id ID] [--wait SECONDS | --no-wait]
herdr-desktop-pet dialogue reset-character --target JSON [--baseline JSON] [--operation-id ID] [--wait SECONDS | --no-wait]
herdr-desktop-pet dialogue status --instance ID --operation-id OPID
```

`KEY` is `idle|running|waiting|unknown|head_tap|body_tap|pet|completion_observed`. `list` prints `{"targets":[{"identity":{"target":{"kind":"character|external_assets","id":"…"},"reference":{"id":"…","revision":N}|null,"generation":N},"name":"…","locales":[…],"slots":[…]}]}`. **Pass the entire serialized `identity` as opaque `--target JSON`**, not just its nested target ID; there is no separate `--ref` flag. The pack reference/revision and registry generation guard stale sources. `get` prints `{selection:{identity,locale,slot},authored:string|null,override:string|null,effective:string|null,baseline:{metadata_token:string,override_entry:string|null,target_overrides_token:string},active:boolean}`. `authored` is source text, `override` saved user text, `effective` resolved text (including host reaction fallback where applicable), and `active` means this target affects the current runtime character. Baseline JSON is the entire returned `baseline`, **not** `selection` or `target`. Omit it to perform a fresh read followed by one CAS mutation; no automatic mutation retry or blind overwrite. Entry set/reset compares source `metadata_token` plus that locale/slot's `override_entry`; an unrelated override in the same character does not conflict. Whole-character reset compares source metadata plus `target_overrides_token`, the hash of all that target's overrides; another character's override does not conflict. Source hash binds character identity, referenced pack revision/generation and authored locales; stale source/entry/target report typed `metadata_conflict`, `entry_conflict`, `target_conflict`, or `stale_target`. `reset-entry` clears only one override, `reset-character` clears that character's overrides, not authored pack dialogue. Empty or whitespace-only set clears that entry; nonblank text preserves raw spaces/newlines/tabs. Dialogue text is limited to **2048 UTF-8 bytes**; disallowed control characters fail. File/stdin are read on the client. A saved inactive target can report `committed:true,native_applied:false,active:false` and still be `applied`; do not equate `native_applied:false` with failed persistence. Active saves can remain pending native refresh. Save-first failure preserves the previous committed override/runtime view, with no ghost entry. The CLI neither discards a GUI editor draft/selection nor opens its modal feedback.
For `character(id)`, saved overrides are shared across that ID's authored revisions, while `--target` and CAS still pin the exact authored reference/generation. `active:true` means that shared target affects the currently selected character, **not** that the returned authored revision is selected: reading retained @1 while @2 is selected returns @1 authored/baseline but can still be active. Saving/resetting @1 can update @2's live bubble; `native_applied:true` requires the actual active renderer and the committed shared override to be accepted, not merely a successful save. Changes to the exact authored source, active renderer or same-target override can supersede pending native proof; metadata cache eviction or another target's override cannot. The GUI retains an exact `(target, reference)` selection, including an unavailable removed revision and its draft rather than silently switching revisions. Initial ready editor content hydrates even while focused if untouched; edits made before metadata (including raw whitespace and marked text) remain drafts, and deferred IME unmark gets a settling pass without overwriting later focus, selection, or undo. Cached metadata errors are not retried by polling: explicit editor reopen/selection or a fresh `dialogue get`/read can retry once; loading/successful reads coalesce.

Non-destructive example (replace both quoted JSON values with the exact `identity` and `baseline` from current `list`/`get`):

```sh
herdr-desktop-pet dialogue list
herdr-desktop-pet dialogue get --target '<identity JSON from dialogue list>' --locale en --slot idle
```

### Worktree removal

```text
herdr-desktop-pet worktree inspect --instance ID --source N --generation N --terminal ID
herdr-desktop-pet worktree remove --token TOKEN [--operation-id ID] [--wait SECONDS | --no-wait]
herdr-desktop-pet worktree status --instance ID --operation-id OPID
```

`inspect` requires an **explicit four-part session key**, eligible live local linked checkout (not the main repo/root); it prints `{token,instance_id,expires_in_seconds:60,key:{instance_id,source_id,generation,terminal_id},workspace_id,pane_id,worktree:{repo_key,repo_root,checkout_path,branch:null,is_main:false,is_linked:true}}`. Unlike session listing, this intentionally discloses the destructive checkout path for review. `branch:null` means branch is not known from the upstream information, not that there is no branch. Token is locally fresh, expires after 60 seconds, one-use, frozen to the checked target and invalidated by restart/source change; inspect again after expiry. `remove` accepts **only** the token, not path/workspace ID/force. It consumes the token and rechecks identity; wrong generation or changed target fails. There is no `--force` override, Trash/undo, branch deletion, or guarantee that ignored files are preserved: upstream `force:false` rejects dirty tracked/untracked work but ignored files may still be lost. The branch remains. Review checkout contents, especially ignored files, and use only a disposable checkout you own. The backend removal has a CAS gap; the token/recheck cannot certify the filesystem against independent concurrent changes.
`expires_in_seconds:60` is the **initial lifetime when inspect issues the token**, not a refreshed countdown on cached operation status. Repeated status reads neither renew nor extend the token.

**Destructive example — do not execute until you own and have reviewed a disposable linked checkout.** The angle-bracket values are placeholders, not a first-row auto-selection or a runnable deletion script:

```sh
herdr-desktop-pet worktree inspect --instance '<owned-disposable-instance>' --source '<owned-disposable-source>' --generation '<owned-disposable-generation>' --terminal '<owned-disposable-terminal>'
# Review the exact worktree.checkout_path, repository root, workspace and ignored files first.
herdr-desktop-pet worktree remove --token '<fresh-token-from-reviewed-inspect>' --operation-id '<fresh-unique-operation-id>' --no-wait
herdr-desktop-pet worktree status --instance '<same-instance>' --operation-id '<same-operation-id>'
```

Removal `result` first has `{target:{key,workspace_id,pane_id,worktree},acknowledged:false,final_observed:false}`. Upstream ACK can make it `pending,committed:true,native_applied:false,acknowledged:true,final_observed:false`; that ACK does not prove checkout deletion. `applied` requires a later stamped coherent observation at the right source/generation with **both pane and session absent**; then `result.final_observed:true` and an `observation` are included. Even that is a watcher observation, **not filesystem certification**, and `native_applied` stays false. Upstream refusal, uncertain delivery, missing watcher, expiry, or daemon restart must not be treated as safe-to-resend or proof of disk state. Check the checkout independently if physical deletion matters.

### Pack commands and existing boundaries

`pack list`, `pack import --path PATH`, `pack validate --path PATH`, `pack preview --path PATH --output PATH [--phase idle|running|waiting|unknown] [--time-ms N] [--reaction head_tap|body_tap|pet|completion_observed] [--reaction-age-ms N] [--hit-overlay]`, `pack export ID [--revision N] --output PATH`, `pack select ID`, `pack update ID --path DIR`, `pack restore ID --revision N`, `pack remove ID`, and `pack status OPID` remain separate from dialogue editing. Mutation flags are `[--operation-id ID] [--expected-generation N] [--wait SECONDS | --no-wait]`; **select/restore IDs are positional** (no `--id`; select has no `--revision`, restore requires `--revision`). `--expected-generation` is a CAS guard. Pack output is its own `{operation_id,state,committed,ui_applied,generation,error}` operation, with states including `accepted`, `preparing`, `applying`, `completed`, `failed`, `canceled`, `durability_unknown`, `committed_pending_apply`, `unknown`; `ui_applied` differs from committed store state. With no daemon, default pack mutation executes synchronously in its own offline worker; explicit `--wait` or `--no-wait` requires a **running** daemon and does not secretly start one. An uncertain pack disconnect calls for `pack status OPID`, not a resend. Neither pack preparation/ACK nor a session prompt ACK certifies downstream agent completion.
With `--no-wait`, an initial pack reply already in a failed terminal state exits nonzero even without status polling; only accepted/preparing/applying is a pending ACK. Explicit waits use the post-ACK absolute deadline described above.

## 한국어

### 연결, 식별자, 작업 상태

이 문서는 이전에 배포된 `0.2.0`/`0.2.1` 및 과거 베타 바이너리의 기능 보증이 아니라 **v0.3.3 안정판 소스·기본/사전 빌드 대상의 CLI 계약**입니다. 소스 매니페스트·문서만으로 공개 압축 파일·설치기·포뮬러의 실제 제공 여부가 입증되지는 않습니다. `--source`로 이 체크아웃을 빌드하거나, 사전 빌드를 쓰기 전에 비초안 v0.3.3 Release의 완전한 자산과 갱신된 설치기/포뮬러를 확인하세요. CLI 버전뿐 아니라 **실행 중인 데몬의 실행 파일**도 확인해야 합니다. 아래 명령은 해당 빌드의 `herdr-desktop-pet`을 사용하며, 별도 설명이 없으면 실행 중인 데스크톱 데몬이 필요합니다. JSON 예시는 붙여 넣을 ID가 아니라 필드와 타입을 보여 줍니다. 어떤 명령도 메뉴 이미지를 가져오지 않으며, GUI 초안 편집·테스트 API나 물리 입력 인증도 제공하지 않습니다.

`presentation`, `preferences`, `sessions`, `dialogue`, `worktree`는 동일 UID만 접근하는 `<state-dir>/control.sock` 전용 Unix 소켓을 사용합니다. 데몬과 같은 `--config-dir PATH`, `--state-dir PATH`를 사용하십시오. **`--socket PATH`는 이 제어 소켓이 아니라 Herdr 백엔드를 지정합니다.** HTTP, CLI `--file` 입력의 데몬 측 읽기, 자동 데몬 시작, 셸 대체 경로, 오프라인 변경은 없습니다. `ensure`, `start`, `stop`, `restart`, `settings get`, `settings set auto_start on|off`, `settings set exit_with_herdr on|off`의 **생명주기 설정**과 `preferences set`은 다릅니다. `status`는 데몬/생명주기 상태를 보고합니다. 기존 show/hide/bubble/scale 즉시 제어와 절대값 `presentation set`도 구분하십시오.

`instance_id`는 데몬 생존 기간의 nonce입니다. `presentation get` 등의 읽기 결과에서 얻고 재시작 후 재사용하지 마십시오. 세션 키는 `(instance_id, source_id, generation, terminal_id)` **네 요소 전체**입니다. 소스·터미널만으로, 또는 GUI의 첫 행 선택으로 대상을 추측하지 마십시오. 변경 작업의 `operation_id`는 생략하면 생성됩니다. ACK 손실에 대비하려면 **전송 전에** 고유 `--operation-id ID`를 지정하십시오. 연결 끊김/시간 초과 후 재전송하지 말고 같은 계열의 상태를 조회하십시오. `presentation status OPID --instance ID`와 `pack status OPID`의 OPID는 위치 인자입니다. `preferences|sessions|dialogue|worktree status --instance ID --operation-id OPID`는 각각 자기 계열 작업만 조회합니다. 재시작·만료·unknown 상태가 원래 변경이 실행되지 않았다는 증거는 아닙니다.

기본 대기 시간은 15초입니다. `--no-wait`의 수락/대기 ACK는 저장 또는 네이티브 적용을 뜻하지 않습니다. **이미 종료된 실패**(팩의 `failed`, `canceled`, `durability_unknown`, `committed_pending_apply`, `unknown` 포함)는 `--no-wait`에도 종료 코드가 0이 아니며 팩 `completed`는 성공입니다. `--wait SECONDS`는 유한합니다(presentation 0–86400초, 나머지 도메인 >0–86400초). 도메인 변경·읽기와 팩 폴링은 각 제출 ACK 뒤 절대 대기 기한을 시작합니다. 변경 전 부분 색상 스냅샷 읽기도 읽기 ACK 뒤 별도 기한을 시작하며 그 안에 끝나지 않으면 변경을 제출하지 않습니다. 각 기한은 sleep과 전체 status RPC(조각 응답 포함)에 적용되며 늦게 도착한 종료 결과를 성공으로 보고하지 않습니다. 도메인 **변경** 기한 초과는 마지막 확인된 pending 작업을 담은 `{"ok":false,"deadline_exceeded":true,"operation":…}`와 status 조회 안내 오류를 출력합니다. 도메인 **읽기** 기한 초과는 이 JSON 필드 없이 pending/status 조회 안내 오류를 반환합니다. presentation의 변경/status 봉투에는 `deadline_exceeded`가 있습니다. 기한 전에 발생한 전송/프로토콜 실패는 기한 초과가 아니라 전달 불명으로 보고합니다. 시간 초과는 취소·롤백·재전송하지 않습니다. `state`, `committed`, `native_applied`, `result`를 별도로 해석하십시오. ACK 또는 성공 종료만으로 에이전트 답변, 실제 파일 삭제, AppKit 적용이 인증되지는 않습니다.

도메인 변경 출력은 `{"ok":boolean,"operation":{"instance_id":string,"operation_id":string,"kind":string,"state":string,"committed":boolean,"native_applied":boolean,"result":object|null,"error_code":string|null,"error":string|null}}`입니다. `state`는 `accepted`, `pending`, `applied`, `agent_prompted`, `failed`, `unknown_delivery`, `superseded`, `shutdown` 중 하나이며, `agent_prompted`는 프롬프트 전달 ACK이지 모델 완료가 아닙니다. 계열별 status도 같은 래퍼를 반환하고 실패 종료 상태는 오류를 반환합니다. 거절된 요청에는 작업 객체가 없을 수 있습니다. `--no-wait`의 `ok:true`여도 아직 pending일 수 있습니다. 세션/대화/워크트리 읽기는 래퍼가 아닌 결과 객체를 직접 출력합니다. 관측 전 필드는 null일 수 있습니다.

### 앱 업데이트 상태와 복구

```text
herdr-desktop-pet update-capabilities
herdr-desktop-pet update-status [OPID] --state-dir PATH
```

데몬 없이 가능한 읽기 전용 명령입니다. 이 체크아웃의 `update-capabilities`는 `{"protocol":2}`를 출력합니다. 업데이트 ID는 소문자 16진수 32자리이며 다른 도메인 작업 ID와 다릅니다. ID를 생략하면 마지막 결과보다 진행 중 예약을 먼저 조회합니다. protocol 2의 계획과 저널에는 모두 `version:2`가 필수이고 저널에는 `execution_fence` (`no_spawn`, `manager_intent`, `manager_exited`)가 필수입니다. `no_spawn`은 관리자 흔적이 없다는 증거가 함께 있어야 미실행을 뜻합니다. `manager_intent`는 실행 전에 저장되므로 PID/종료 정보가 없어도 **불명확**한 상태이지 재시도 허가가 아닙니다. `manager_exited`에는 종료와 출력 배출 증거가 필요합니다. phase·fence·`installed`·`applied`는 서로 다른 사실입니다. 오프라인 `update-status`는 저장된 이력·계획/오류와 가능할 때 남은 도우미의 정확한 argv를 보여줄 뿐 현재 프로세스를 확인하거나 도우미를 실행하거나 서명을 검증하지 않습니다. 안전하게 조정된 결과도 이력상 `Failed`일 수 있으며 실패 종료 코드는 예약이 여전히 남았다는 뜻이 아닙니다.

남은 private 도우미의 코드 서명을 검증한 뒤 그 도우미의 `status --state-dir PATH --operation-id OPID`로 현재 도우미·관리자·후보 증거를 확인하세요. 명시적 `recover --state-dir PATH --operation-id OPID [--start]`는 작업별 lease를 유지하며 기존 도우미가 lease를 점유한 동안 복구 요청을 거절합니다. 설치를 반복하지 않고 조정합니다. `no_spawn`과 관리자 흔적 부재, 정확한 기존 PID·인스턴스·이미지·소스·프로필의 준비 완료가 입증되면 예약을 해제하면서 `Failed`, `installed:false`, `applied:false`로 기록할 수 있습니다. 변경 없는 원본이 종료된 경우 시작하지 않고 조정할 수도 있습니다. 새로 준비된 원본 인스턴스는 원본 종료 검사보다 먼저 확인하며 업그레이드가 아니라 `Failed`입니다. 검증된 설치 후보 시작이나 검증된 원본 복원은 명시적 `--start`에서만 허용하며 설치와 적용은 다릅니다. 일부 예약 표식 해제는 반복해도 안전하게 조정합니다. 해제가 불완전/불명확하거나 관리자·그룹·출력·증거를 입증할 수 없거나 다른 프로필·동일 설치 범위의 실행 사용자·더 최신 사용자 Stop이 있으면 안전을 우선합니다. `Unknown`은 예약을 유지하고 설치를 재실행하지 않습니다. 표식을 삭제하거나 원래 설치를 맹목적으로 재시도하지 마세요. 안전하게 정리된 실패 뒤 새 확인과 사용자 동의는 **별개의 새 작업**이지 이전 설치의 재생이 아닙니다. `run`은 앱의 서명된 도우미 인계용이며 범용 설치 CLI가 아닙니다.

기존 작업의 `run`은 실행 fence나 lifecycle 증거를 바꾸기 전에 이미 있는 journal을 거절합니다. 설치 재실행이 아닌 `recover`를 사용하세요. 외부 설치가 검증된 후보도 명시적 `recover --start`나 우선하는 사용자 Stop 전까지 `Failed`, `installed:true`, `applied:false`와 예약이 남을 수 있습니다. 수락된 Stop 응답을 읽지 않았다는 사실만으로 서버의 응답 쓰기 실패를 입증할 수는 없습니다.

protocol 1 계획·저널·예약 표식은 차단하고 보존하며 변환·기본값 대입·별칭 적용·무단 삭제하지 않습니다. 관리형 Herdr의 원본/ref와 처음 주입된 호스트 플러그인 설정 경로는 플러그인 하위 디렉터리, 소켓, 덮어쓴 프로필과 구분합니다. 설치 충돌 범위는 Herdr checkout 루트, 로컬 루트 또는 Homebrew 포뮬러의 실제 Cellar 루트이며 Brew prefix 전체가 아닙니다. 선택된 프로필과 외부 에셋을 기본값으로 바꾸지 않고 유지합니다. 사용자 Stop은 언제나 업데이트 재시작보다 우선합니다.

### 표시 상태 (presentation)

```text
herdr-desktop-pet presentation get
herdr-desktop-pet presentation set (--visible on|off | --passthrough on|off | --alpha-passthrough on|off | --bubble-visible on|off | --bubble-placement above|below|left|right|auto | --scale NUMBER) [서로 다른 필드 추가 가능] [--expected-revision N] [--operation-id ID] [--wait SECONDS | --no-wait]
herdr-desktop-pet presentation reset [--expected-revision N] [--operation-id ID] [--wait SECONDS | --no-wait]
herdr-desktop-pet presentation status OPID --instance ID
```

`set`에는 최소 한 개의 절대값 필드가 필요합니다. `--scale`은 유한한 절대 렌더러 배율이며 `bigger`/`smaller`의 상대 조정이 아닙니다. `reset`은 캐릭터와 독립 말풍선의 위치만 초기화하고 표시 여부를 보존합니다. `get`의 `{ok:true,instance_id,snapshot}`에서 snapshot에는 `instance_id`, `revision`, `desired`, `effective|null`, `persisted|null`, `pet_window_visible|null`, `bubble_window_visible|null`, `pet_window_frame|null`, `bubble_window_frame|null`, `pending_reasons`가 있습니다. 프레임은 `{x,y,width,height}`이고 실제 창 상태는 목표보다 늦을 수 있습니다. 변경/status는 `{ok,instance_id,operation:{instance_id,operation_id,state,revision,target,native_applied,persisted,error},deadline_exceeded}`를 반환합니다. presentation 상태는 `accepted|pending|applied|superseded|persist_failed|rejected|shutdown|unknown`이며 초기 거절/불확실 전송에는 작업 객체 대신 `state:"rejected"`/`"unknown"`, ID와 오류가 나옵니다. `--expected-revision`은 CAS 조건입니다. 표시 변경은 **런타임 먼저** 진행되므로 네이티브 적용과 정확한 대상의 저장 성공을 분리해 확인합니다. `persist_failed`를 네이티브 변화로 감추거나 다른 저장으로 이 작업을 인증하지 않습니다. 저장되지 않은 후보를 유령 저장값으로 취급하지 마십시오.
수락된 표시 변경이 한 배치에 모이면 표시 여부·클릭 통과·알파 클릭 통과·말풍선 표시·배치·배율 중 명시적으로 요청한 필드를 최종 정규화 장면에서 저장합니다. 같은 대상의 후속 no-op도 이를 잃지 않지만, 거절된 후보는 다음 말풍선 전용 저장에 섞이지 않습니다. 다른 전체 대상에 의한 기존 superseded 규칙은 그대로입니다.

### 환경 설정 (preferences)

```text
herdr-desktop-pet preferences get
herdr-desktop-pet preferences set [--language system|ko|en] [--theme warm_ivory|dusty_rose|moonlit_ink|custom] [--surface #RRGGBB] [--text #RRGGBB] [--muted #RRGGBB] [--border #RRGGBB] [--accent #RRGGBB] [--status-indicators on|off] [--menu-bar always|recovery_only] [--observation-local on|off] [--observation-remote on|off] [--machine ID ... | --clear-machines] [--expected-revision N] [--operation-id ID] [--wait SECONDS | --no-wait]
herdr-desktop-pet preferences status --instance ID --operation-id OPID
```

`set`에는 설정 하나 이상이 필요합니다. `--machine`을 반복해 카탈로그 ID를 지정하며 `--clear-machines`와 함께 쓸 수 없습니다. 알 수 없거나 비활성화된 ID는 `unknown_machine` 등 타입이 있는 `error_code`로 실패합니다. 먼저 카탈로그를 확인하십시오. 색상 일부만 지정하면 저장된 팔레트를 읽어 지정한 색만 병합하고 그 읽기의 revision으로 쓰기를 보호합니다. 경쟁 변경은 덮어쓰지 않고 실패합니다. `--expected-revision`의 경쟁 설정 변경은 `revision_conflict`입니다. `get`은 도메인 래퍼의 `operation.result`에 `revision`, `desired`, `persisted`, `effective`, `pending_reasons`, `status_item_visible`, `effective_locale`, `observation_catalog`를 담습니다. 카탈로그에는 `initialized` 및 각 머신의 ID·라벨·enabled·status가 있습니다. 스냅샷 설정 키는 `language`, `bubble_appearance`, `show_status_indicators`, `menu_bar_mode`, `observation_local`, `observation_remote`, `observation_machines`입니다. 환경 설정은 **저장 먼저** 수행합니다. 저장 실패는 이전 설정/런타임을 유지하며 유령 후보를 만들지 않습니다. `committed:true`는 저장됨, `native_applied`와 pending 이유는 그 뒤 UI 상태입니다. 생명주기 `settings`와 혼동하지 마십시오.
CLI `--language`는 분리형/`--language=VALUE` 모두 **정확히** `system`, `ko`, `en`만 허용하며 오타·대소문자 변경·빈 값·앞뒤 공백은 프로필/소켓/저장소 접근 전에 거부합니다. 디스크의 기존 알 수 없는 언어값을 읽는 관대한 fallback은 유지됩니다. CLI `--machine`은 **전체 목록 교체**이므로 모든 ID가 현재 활성화되어야 합니다. GUI의 신뢰된 선택 변경만 저장된 누락/비활성 ID를 유지·제거하고 현재 활성 ID를 추가할 수 있으며 CLI 검사를 완화하지 않습니다. 조용한 상태에서 열린 색상 설정은 충돌/조합 상태에 따른 버튼 자격·라벨만 갱신하고 필드 초안·포커스·선택·실행 취소·색상 견본은 건드리지 않으며 창을 닫으면 갱신을 중단합니다.

### 세션과 에이전트 프롬프트

```text
herdr-desktop-pet sessions list --instance ID [--filter all|idle|working|waiting|completed|unknown|offline] [--limit 1..128]
herdr-desktop-pet sessions show --instance ID --source N --generation N --terminal ID
herdr-desktop-pet sessions prompt --instance ID --source N --generation N --terminal ID (--text TEXT | --file PATH | --stdin) [--operation-id ID] [--wait SECONDS | --no-wait]
herdr-desktop-pet sessions status --instance ID --operation-id OPID
```

`list`는 `{instance_id,revision,filter,total,matched,status_summary,rows,next_cursor:null}` 전체 모음 JSON을 출력합니다. `--limit` 기본값은 32, 허용 범위는 1–128이며 **전체 행 한도가 아닌 페이지 크기**입니다. CLI가 128행 너머까지 모든 페이지를 수집하고 instance/revision/filter/개수를 검증합니다. 불일치 시 부분 결과를 성공으로 출력하지 않습니다. 각 행과 `show`는 `key:{instance_id,source_id,generation,terminal_id}`, `source_label`, `is_local`, `pane_id`, `availability`, `status`, `outcome`, `display_status`, `metadata`(제목, 에이전트, workspace/tab ID와 라벨)를 포함합니다. 원본 소스 소켓 경로, 작업 디렉터리, 워크트리 checkout/root 경로는 제외됩니다. `show`는 정확한 키로 보존된 오프라인 행도 읽을 수 있지만 프롬프트 가능하다는 뜻은 아닙니다. `prompt`는 살아 있는 적격 로컬 소스만 대상으로 하며 원격/읽기 전용/오래된/오프라인 세션을 대상으로 하지 않습니다. `--text`, `--file`, `--stdin` 중 하나만 선택합니다. 파일과 표준 입력은 **클라이언트가** UTF-8로 읽고 데몬에는 텍스트를 보냅니다. 원본 프롬프트에는 512 KiB 예산(클라이언트의 4096바이트 예약 후 최대 **520192 UTF-8 바이트**)과 별도 큰 요청 봉투가 적용됩니다. 일반 도메인 요청/결과 프레임 한도는 각각 16 KiB/64 KiB, 프롬프트용 봉투/큰 응답은 약 516 KiB/68 KiB입니다. `agent_prompted`는 상위 프롬프트 ACK만 의미합니다. 모델의 정확한 완료는 세션 상태/결과를 별도로 관측하십시오.
데몬의 각 `sessions list` 페이지는 일반 도메인 결과 64 KiB 한도와 별도로 **봉투·JSON 이스케이프·커서·마지막 줄바꿈까지 인코딩된 응답 전체 512 KiB**로 제한됩니다. `--limit` 이하에서 들어갈 수 있는 가장 긴 행 접두부를 반환하며 행 하나와 필요한 커서도 들어가지 않으면 행/메타데이터 절단이나 빈 연속 페이지 대신 한도 내 오류를 반환합니다. CLI의 여러 페이지 수집 결과 전체는 512 KiB를 넘을 수 있습니다.

읽기 전용 예시(반환된 instance와 행의 정확한 키를 직접 옮기십시오):

```sh
herdr-desktop-pet presentation get
herdr-desktop-pet sessions list --instance '<get에서 얻은 instance>' --filter all --limit 32
herdr-desktop-pet sessions show --instance '<row.key.instance_id>' --source '<row.key.source_id>' --generation '<row.key.generation>' --terminal '<row.key.terminal_id>'
```

### 대화문 (dialogue)

```text
herdr-desktop-pet dialogue list
herdr-desktop-pet dialogue get --target JSON --locale ko|en --slot KEY
herdr-desktop-pet dialogue set --target JSON --locale ko|en --slot KEY (--text TEXT | --file PATH | --stdin) [--baseline JSON] [--operation-id ID] [--wait SECONDS | --no-wait]
herdr-desktop-pet dialogue reset-entry --target JSON --locale ko|en --slot KEY [--baseline JSON] [--operation-id ID] [--wait SECONDS | --no-wait]
herdr-desktop-pet dialogue reset-character --target JSON [--baseline JSON] [--operation-id ID] [--wait SECONDS | --no-wait]
herdr-desktop-pet dialogue status --instance ID --operation-id OPID
```

`KEY`는 `idle|running|waiting|unknown|head_tap|body_tap|pet|completion_observed`입니다. `list` 결과는 `{"targets":[{"identity":{"target":{"kind":"character|external_assets","id":"…"},"reference":{"id":"…","revision":N}|null,"generation":N},"name":"…","locales":[…],"slots":[…]}]}` 형태입니다. `--target JSON`에는 **반환된 `identity` 전체를 불투명한 JSON 값으로** 전달하십시오. 내부 target ID만 전달하지 말고 별도의 `--ref` 옵션도 없습니다. 참조 팩 revision과 등록 generation이 오래된 소스를 구별합니다. `get`은 `{selection:{identity,locale,slot},authored:string|null,override:string|null,effective:string|null,baseline:{metadata_token:string,override_entry:string|null,target_overrides_token:string},active:boolean}`을 반환합니다. `authored`는 원본, `override`는 사용자 저장값, `effective`는 호스트 반응 기본 문구까지 고려한 실제 해석값입니다. `active`는 이 대상이 현재 런타임 캐릭터에 영향을 준다는 뜻입니다. `--baseline JSON`에는 반환된 **`baseline` 전체**를 넣고 `selection`/`target`만 넣지 마십시오. 생략하면 새로 읽은 뒤 단 한 번 CAS 변경하며 변경 자동 재시도/무조건 덮어쓰기는 없습니다. 항목 set/reset은 소스 `metadata_token`과 해당 언어·슬롯의 `override_entry`를 비교하므로 같은 캐릭터의 다른 항목 변경은 충돌하지 않습니다. 캐릭터 전체 reset은 소스 메타데이터와 해당 대상 모든 override의 `target_overrides_token` 해시를 비교하며 다른 캐릭터 변경은 충돌하지 않습니다. 소스 해시는 캐릭터 식별자, 참조 팩 revision/generation, 작성된 모든 언어의 내용을 포함합니다. 오래된 소스/항목/대상은 `metadata_conflict`, `entry_conflict`, `target_conflict`, `stale_target` 등의 코드로 실패합니다. `reset-entry`는 한 override, `reset-character`는 해당 캐릭터의 override만 지우며 팩 원본 대화문은 지우지 않습니다. 빈 문자열·공백만 있는 set은 항목을 비웁니다. 공백 외 내용은 원본 공백/줄바꿈/탭을 보존합니다. 대화문은 최대 **2048 UTF-8 바이트**이고 허용되지 않는 제어 문자는 거부합니다. 파일/표준 입력은 클라이언트가 읽습니다. 비활성 대상 저장은 `committed:true,native_applied:false,active:false`이면서 `applied`일 수 있습니다. 네이티브 false를 저장 실패로 해석하지 마십시오. 활성 저장은 네이티브 갱신을 기다리며 pending일 수 있습니다. 저장 우선 실패는 이전 override/런타임을 유지하며 유령 항목이 없습니다. CLI는 GUI 편집기 초안/선택을 지우거나 모달을 열지 않습니다.
`character(id)`의 저장 override는 같은 ID의 모든 작성 revision에 공유되지만 `--target`과 CAS는 정확한 작성 참조/generation을 지정합니다. `active:true`는 반환된 작성 revision 자체가 선택되었다는 뜻이 아니라 그 공유 대상이 현재 선택 캐릭터에 영향을 준다는 뜻입니다. @2가 선택된 동안 보존된 @1을 읽으면 @1의 원문/기준값과 `active:true`가 함께 나올 수 있습니다. @1 저장/초기화도 @2의 실제 말풍선에 반영될 수 있고, `native_applied:true`는 저장 성공만이 아닌 현재 렌더러와 저장된 공유 override의 실제 수락을 요구합니다. 정확한 작성 소스·활성 렌더러·동일 대상 override의 변경은 보류 중인 네이티브 증거를 supersede할 수 있지만 메타데이터 캐시 퇴출이나 다른 대상 변경은 그렇지 않습니다. GUI는 정확한 `(target, reference)` 선택을 보존하고 삭제된 revision도 사용 불가 상태와 초안을 유지하며 몰래 다른 revision으로 바꾸지 않습니다. 손대지 않은 첫 ready 편집 내용은 포커스 중에도 채우며 metadata 도착 전 실제 편집(원본 공백·조합 문자 포함)은 초안으로 유지합니다. 조합 종료 뒤 미뤄진 초기 채우기를 한 번 처리하되 이후 포커스·선택·실행 취소는 덮어쓰지 않습니다. 캐시된 metadata 오류는 자동 polling으로 재시도하지 않고 명시적 편집기 재열기/선택이나 새 `dialogue get`/읽기에서 한 번 재시도하며 진행 중/성공 결과는 합칩니다.

비파괴 예시(`identity`와 `baseline`은 현재 `list`/`get`에서 얻은 JSON만 사용):

```sh
herdr-desktop-pet dialogue list
herdr-desktop-pet dialogue get --target '<dialogue list의 identity JSON>' --locale en --slot idle
```

### 워크트리 제거

```text
herdr-desktop-pet worktree inspect --instance ID --source N --generation N --terminal ID
herdr-desktop-pet worktree remove --token TOKEN [--operation-id ID] [--wait SECONDS | --no-wait]
herdr-desktop-pet worktree status --instance ID --operation-id OPID
```

`inspect`에는 **명시적 네 요소 세션 키**가 필요하며, 살아 있는 로컬 연결 워크트리만 허용합니다. 메인 저장소/루트는 보호됩니다. 결과는 `{token,instance_id,expires_in_seconds:60,key:{instance_id,source_id,generation,terminal_id},workspace_id,pane_id,worktree:{repo_key,repo_root,checkout_path,branch:null,is_main:false,is_linked:true}}`입니다. 세션 목록과 달리 여기에는 삭제 전 검토할 정확한 checkout 경로가 노출됩니다. `branch:null`은 상위 정보에 브랜치가 없어 알 수 없다는 뜻이지 브랜치가 없다는 뜻이 아닙니다. 토큰은 로컬에서 새로 생성되고 60초 만료, 일회용, 확인한 대상에 고정되며 재시작/소스 변경 시 무효가 됩니다. 만료되면 다시 inspect하십시오. `remove`는 토큰 **하나로만** 대상을 지정합니다. 경로/workspace ID/force 옵션이 없으며 토큰 사용 시 식별자를 다시 확인합니다. generation 불일치/대상 변경은 실패합니다. `--force` 우회, 휴지통/실행 취소, 브랜치 삭제는 없습니다. 상위 `force:false`는 변경된 tracked/untracked 파일을 거절하지만 **ignored 파일은 삭제될 수 있습니다**. 브랜치는 남습니다. 자기 소유의 폐기 가능한 checkout에서만, ignored 파일까지 확인한 뒤 실행하십시오. 백엔드 제거 사이에는 CAS 간극이 있으므로 토큰 재확인이 외부의 동시 파일시스템 변경까지 보증하지는 않습니다.
`expires_in_seconds:60`은 inspect가 토큰을 발급한 **시점의 초기 유효 시간**이며, 캐시된 작업 status에서 새로 계산한 잔여 시간이 아닙니다. status 반복 조회로 토큰이 갱신되거나 연장되지 않습니다.

**파괴적 예시 — 본인 소유의 폐기 가능한 연결 checkout과 그 내용을 검토하기 전에는 실행하지 마십시오.** 꺾쇠 값은 명시적 자리표시자이며 첫 세션 자동 선택/자동 삭제 스크립트가 아닙니다:

```sh
herdr-desktop-pet worktree inspect --instance '<본인 소유 폐기 대상 instance>' --source '<본인 소유 폐기 대상 source>' --generation '<본인 소유 폐기 대상 generation>' --terminal '<본인 소유 폐기 대상 terminal>'
# 정확한 worktree.checkout_path, 저장소 루트, workspace, ignored 파일을 먼저 확인하십시오.
herdr-desktop-pet worktree remove --token '<검토한 inspect의 새 token>' --operation-id '<새 고유 operation ID>' --no-wait
herdr-desktop-pet worktree status --instance '<같은 instance>' --operation-id '<같은 operation ID>'
```

제거 초기 `result`는 `{target:{key,workspace_id,pane_id,worktree},acknowledged:false,final_observed:false}`입니다. 상위 ACK 뒤에는 `pending,committed:true,native_applied:false,acknowledged:true,final_observed:false`가 될 수 있으며 이는 checkout 삭제의 증거가 아닙니다. 올바른 소스/generation의 일관된 새 관측에서 **pane과 session이 모두 없음**을 확인해야 `applied`, `result.final_observed:true`, `observation`이 됩니다. 이것도 감시자 관측이지 **파일시스템 인증이 아니며** `native_applied`는 계속 false입니다. 상위 거절, 불명확한 전달, 감시자 부재, 만료, 데몬 재시작은 재전송 허가나 디스크 상태의 증거가 아닙니다. 물리적 삭제가 중요하면 checkout을 별도로 확인하십시오.

### 팩 명령 및 기존 경계

`pack list`, `pack import --path PATH`, `pack validate --path PATH`, `pack preview --path PATH --output PATH [--phase idle|running|waiting|unknown] [--time-ms N] [--reaction head_tap|body_tap|pet|completion_observed] [--reaction-age-ms N] [--hit-overlay]`, `pack export ID [--revision N] --output PATH`, `pack select ID`, `pack update ID --path DIR`, `pack restore ID --revision N`, `pack remove ID`, `pack status OPID`는 대화문 편집과 별개입니다. 변경 옵션은 `[--operation-id ID] [--expected-generation N] [--wait SECONDS | --no-wait]`입니다. **select/restore의 ID는 위치 인자**이며 `--id`가 없습니다. select에는 `--revision`이 없고 restore에는 필수입니다. `--expected-generation`은 CAS 보호입니다. 팩 작업 출력은 별도의 `{operation_id,state,committed,ui_applied,generation,error}`이고 상태에는 `accepted`, `preparing`, `applying`, `completed`, `failed`, `canceled`, `durability_unknown`, `committed_pending_apply`, `unknown` 등이 있습니다. `ui_applied`와 저장소 commit은 다릅니다. 데몬이 없으면 기본 팩 변경은 자체 오프라인 worker에서 동기 실행됩니다. 명시적 `--wait`/`--no-wait`는 **실행 중인 데몬**이 필요하며 몰래 시작하지 않습니다. 불명확한 연결 끊김은 재전송 대신 `pack status OPID`로 조사하십시오. 팩 준비/ACK와 세션 프롬프트 ACK 어느 쪽도 하위 에이전트 완료를 인증하지 않습니다.
`--no-wait`라도 팩의 첫 응답이 이미 실패 종료 상태이면 status 폴링 없이 종료 코드가 0이 아닙니다. `accepted`/`preparing`/`applying`만 대기 ACK입니다. 명시적 대기는 위의 ACK 이후 절대 기한을 따릅니다.
