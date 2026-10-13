# 개발

[English](development.md) · **한국어** · [설치와 개요](../readme.ko.md)

아래 명령은 저장소 루트에서 실행합니다. [v0.3.6](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/v0.3.6)은 검증된 arm64 자산·체크섬과 안정판 Homebrew 포뮬러를 갖춘 최신 공개 안정판입니다. 소비자 설치에는 소스 체크아웃이 필요 없으며 아래 소스 개발은 선택 사항입니다. 지원 플랫폼은 Apple Silicon macOS 13 이상이며 Intel Mac·Windows·Linux용 네이티브 배포는 없습니다.

**공개 소비자 경로 검증:** 실제 안정판 Brew 0.3.5 → 0.3.6 업그레이드가 성공했으며 공개 고정 `--prebuilt` 설치기는 시스템 도구만 있는 `PATH`에서 완료됐습니다(Cargo·Bun·Node.js·npm 없음). 두 설치 앱 모두 별도 비공개 Tern config/state 프로필에서 `app_version: 0.3.6`, `observation_mode: tern`, `ui_ready: true`로 시작했고 설정된 Herdr 소켓의 부모 디렉터리도 필요하지 않았습니다. 유효한 빈 exchange-v1 스냅샷은 `command: null`로 수락됐으며 일반 Stop으로 깨끗하게 종료됐습니다. 실제 공개 플러그인 URL로 격리 Tern 설정에 0.1.0을 설치했고 카탈로그는 문제 없이 준비됐습니다. 이는 해당 소비자 경로의 근거이며 GUI 플러그인 창 연결·에이전트 목록/모델 전달·전체 업데이트 거래·일반 인증의 근거는 아닙니다. 기존 사용자 프로필은 변경하지 않았습니다.

## 소스 개발

이 체크아웃의 네이티브 앱을 소스에서 빌드하고 수동 시작하는 데 Herdr 플러그인·패치 호스트·client-attach 훅은 **필요하지 않습니다**. 지원되는 Apple Silicon Mac에서 Rust/Cargo·Bun·Node.js/npm·Xcode Command Line Tools·`codesign`을 준비하고 앱을 패키징하세요. 아래 수동 시작 예시는 정상 로컬 Herdr 서버가 이미 실행 중인 경우입니다:

```sh
bash scripts/install.sh --source
./dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet start
./dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet status
```

기존 로컬 Herdr 서버가 있다면 수동 `start`로 플러그인·호스트 패치 없이 로컬 세션 스냅샷(Herdr 0.9.0 이상)을 볼 수 있습니다. 답장에는 해당 서버의 `agent.prompt` 지원이 별도로 필요합니다. 소켓 지정과 데몬 확인은 Homebrew 명령 대신 위 패키지 CLI를 사용해 [첫 연결 안내](../readme.ko.md#homebrew로-설치-첫-연결)를 따르세요. 정상 Herdr 서버 없이 캐릭터만 실행한다면 [Herdr 없이 캐릭터만 실행](../readme.ko.md#herdr-없이-캐릭터만-실행)처럼 **시작 전에** 해당 CLI에서 `exit_with_herdr off`를 설정하세요. 기본 단절 종료 동작은 바뀌지 않습니다. 아래 별도 호스트 절차는 **선택형** 관리형 플러그인 실행 관리·client-attach 자동 시작 전용이며 앱만 빌드해도 순정 호스트에 `client.attached` 훅이 생기지는 않습니다.

### 로컬 Tern 연동 (사전 빌드 소비자 경로 또는 선택형 소스 개발)

공개 v0.3.6은 `brew install hanbong5938/tap/herdr-desktop-pet`으로 공통 네이티브 앱을 설치하고 별도 공개 [tern-desktop-pet 플러그인](https://github.com/hanbong5938/tern-desktop-pet)을 `tern plugin install https://github.com/hanbong5938/tern-desktop-pet`으로 명시적으로 설치합니다. 이 사전 빌드 경로에는 Rust/Cargo·Bun·Node.js·소스 체크아웃이 필요 없습니다. 플러그인 설치는 앱을 몰래 내려받거나 빌드·실행하지 않습니다. 소스 개발은 위 절차로 현재 체크아웃을 빌드하며 별도 연동 브랜치 전환은 필요하지 않습니다. 개발자는 `tern plugin link "$PWD/../tern-desktop-pet"`으로 형제 플러그인 체크아웃을 연결할 수 있습니다. 두 실행 파일 모두 [Tern 시작·별도 설정/상태 프로필 안내](usage.ko.md)를 따르세요. 기존 Brew 사용자는 `brew update && brew upgrade hanbong5938/tap/herdr-desktop-pet`를 사용하며 프로필·환경설정은 유지됩니다.

이 체크아웃의 동일한 네이티브 앱은 Herdr 서버나 별도 앱 ID 없이 Tern을 직접 지원합니다. 위 소스 패키지 실행 파일에서 한 프로필을 유지하며 `./dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet --host tern start`, `./dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet --host tern status`를 사용하고 `status.observation_mode`가 `tern`인지 확인하세요. `--host both`는 Herdr와 **로컬** Tern을 함께 사용하며 `--socket`은 Herdr 소켓 전용입니다. 명시적 `--host`가 정확한 `HERDR_PET_OBSERVATION_MODE=herdr|tern|both`보다 우선하고, 둘 다 없으면 `herdr`입니다. 실행 중인 데몬에 접속해 모드를 바꿀 수 없습니다. Tern 전용 시작은 Herdr 없이 준비되고 `exit_with_herdr`가 켜져도 Herdr 부재만으로 종료하지 않습니다. 별도 Tern 플러그인은 이 앱의 Herdr 플러그인이 아니며 Tern 그림 제공을 뜻하지 않습니다.

Tern 창 플러그인의 `window.luau` 팔레트 명령에는 실행 파일과 프로필 경로가 설정됩니다. 공개 바이너리가 `PATH`에 있다고 가정하지 말고 이 소스 빌드 CLI와 같은 전용 `--config-dir`/`--state-dir`를 지정하세요. 주기적인 로컬 `agents:list` 목록과 `process.run` 교환은 창마다 주기당 하나의 단명 도우미 프로세스를 실행합니다(이 소스의 주기는 1200 ms). 프로세스 오버헤드는 **측정하지 않았습니다**. 내부 명령 `tern-exchange`는 한도 있는 stdin/stdout JSON을 동일 UID만 허용하는 전용 제어 소켓으로 전달하며([CLI 계약](cli.md) 참고), 원격 Tern 감시자는 없습니다. 창/pane 식별자, 인스턴스/epoch/sequence, 5초 신선도 검사로 오래되거나 재사용된 세션을 막고 정확한 살아 있는 로컬 세션에만 `agents:ask`를 한 번 제시합니다. `accepted`는 ask가 대기열에 있을 수 있다는 뜻이지 모델 전달/완료가 아닙니다. 전달·결과 불명 시 자동 재시도나 PTY 대체가 없습니다. 설치된 Tern 0.6.1 선언에는 완료 이벤트가 없으므로 `exited`는 완료가 아닌 불명입니다.

현재 수정 계약: `status.herdr_socket`은 Tern 전용 모드에서도 고정된 UpdateContext의 설정 경로이며 실제 Herdr 연결의 증거가 아닙니다. 가짜 등록/소켓은 필요 없습니다. endpoint 신원은 빈/상대 경로를 거절하고 endpoint/부모가 없어도 같은 절대 경로를 허용하며, 그 외에는 전체 canonical 별칭 또는 canonical 부모+같은 파일명으로 비교합니다. basename만으로 대체하지 않습니다. 플러그인은 단일 정규 KV `connection={version=1,executable,config_dir,state_dir}`를 저장합니다. 전체 객체 검증 후 한 번 커밋한 다음 프로필/incarnation을 바꾸며 취소·실패는 기존 연결을 보존합니다. 정규 값이 없을 때만 이전 세 키를 한 번 채택하고 성공적인 커밋 뒤 정리합니다. 잘못된 정규 값은 이전 키로 대체하지 않으며 외부 공유 프로필 커밋은 지연 콜백을 차단합니다.

Exchange v1 실패의 선택적 `error_code`는 `binding_required|instance_changed|source_retired|receipt_rejected|rejected`뿐입니다. 첫 미바인딩 목록은 results 없이 보내며 command 없이 응답합니다. 첫 등록 응답 유실은 정상 다음 poll에서 새 창 신원으로 복구하고 `source_retired`는 ask 재전송 없이 신원을 재설정합니다. 정확히 바인딩된 `receipt_rejected`는 해당 요청에 캡처한 보고만 버립니다. 요청 JSON은 별도 256 KiB, 전체 인코딩 응답은 **마지막 LF 포함** 512 KiB입니다. 실제 직렬화로 pending/offer 전에 프롬프트를 허용하거나 확정 TooLarge로 거절하며 Herdr 여유 크기를 추측하지 않습니다. Herdr와 Tern의 공유 소스 한도는 64개입니다. pending 작업을 종결한 뒤 만료된 오프라인 Tern만 회수하고 살아 있는 소스/Herdr는 축출하지 않으며 소스 ID는 재사용하지 않습니다. 제어 accept 루프의 기존 100 ms monotonic tick은 직렬 Herdr poll·클라이언트 I/O와 독립적으로 **5초 이상**에서 신선도를 만료시킵니다.

50 ms SDK 콜백 수정은 신선한 목록을 네 번의 `tern.sleep(0):next` microtask로 나눕니다. tabs·sessions·agents는 각각 별도 콜백, hosts·panes는 마지막 콜백에서 함께 읽고 순수 검증을 수행합니다. 콜백당 목록 API는 최대 두 개(API당 12 ms fixture에서 24 ms)입니다. busy/active-work가 비동기 교환 전체의 incarnation/프로필/단계를 보호하고 최종 중요 local/pane/agent 검사는 일관된 신선한 context를 사용합니다. accepted·failed 보고는 기존의 신선한 전체 목록 교환을 즉시 시작하며 변경 없는 1200 ms 주기 타이머를 기다리지 않습니다. 캐시 목록·ask 재시도·새 프로토콜·네이티브 5초 수락 영수증 기한 증가는 없습니다. 데스크톱 부하에서 1 ms 타이머 콜백 지터가 997 ms까지 측정돼 목록의 coarse timer 인계를 제거했습니다. 개별 SDK 호출이나 OS 선점은 이 예산 모델 밖이며 무기한 지연 보장이나 일반적인 프로세스 오버헤드 벤치마크는 아닙니다.

요청 한도는 정확히 JSON 256 KiB이며 선택적 framing LF/CRLF는 세지 않습니다. 응답은 LF 포함 전체 512 KiB 그대로입니다. 이전 실제 CLI 요청 411918바이트는 수정 전 허용됐지만 이제 IPC 전에 거절합니다. 소스 한도 때문에 입장이 거절된 건강한 Herdr 연결은 유지하고 여유가 생기면 정상 다음 일관된 스냅샷에서 허용합니다. 첫 기준선은 과거 완료를 만들어내지 않으며 별도 재시도 스레드/backoff는 없습니다.

**공개 전 수행한 수정 검증 기록:** 네이티브 674개 통과·1개 무시와 실제 AppKit composer 실행 파일, 코디네이터 70개(라이브러리 49·바이너리 21), TypeScript/Bun 58개, arm64 릴리스/패키지 및 ad-hoc strict/deep 서명 검증을 확인했습니다. 실제 Tern 0.6.1의 생산 `window.luau`를 제어된 SDK로 실행한 **44/44개**가 통과했습니다. 기존 사례, API 예산, coarse timer와 microtask 기한 모델, awaitable 실패 정리 및 신원/프로필/local/pane/host fence를 포함합니다. 독립 모델이 아닌 실제 생산 Luau의 제어된 SDK 근거이며 의미 있는 전달 전체 증거는 아닙니다. 설정 소켓 부모가 없는 서명 Tern 전용 UI 준비와 바인딩/source-retired wire 검사를 확인했습니다. 실제 300 KiB ASCII와 이스케이프 Unicode 원본 300000바이트는 각각 307361·390161바이트 응답으로 보존됐지만 주입한 수락 영수증이지 실제 모델 전달은 아닙니다. 제어 문자 이스케이프 입력은 offer 없이 `prompt_too_large`로 확정 거절했습니다. 게시·커밋은 주장하지 않습니다.

실제 패키지 Tern 데몬의 lifecycle 검사에서 신선한 소스 64개를 허용했고 5.25초 뒤 연결 소스는 0개였습니다. 새 관측을 허용한 뒤 축출된 기존 바인딩 창은 `source_retired`를 반환했습니다. 이는 소스 한도/만료 wire 근거이지 도우미 거래나 의미 있는 ask의 증거는 아닙니다.

실제 서명된 현재 Tern 도우미와 빌드한 legacy base-HEAD 도우미는 설정 소켓 부모가 없어도 staging·사전 검사를 통과했습니다. 현재 도우미는 UI 승인 없는 수동 fixture에서 UI Prepare의 `update plan or operation changed; check again` guard에 도달했습니다. 실제 복구는 정확한 Original NoSpawn을 입증하고 `Failed`/`no_spawn`, `installed:false`, `applied:false`를 기록하며 예약을 해제하고 Original PID/instance/image/source/profile을 그대로 유지했습니다. 서명된 legacy 도우미는 모드 capability 없는 `{"protocol":3}`만 광고했으며 기본 Herdr 키가 없는 상태에서 같은 사전 검사/NoSpawn 복구가 통과했습니다. 앱/도우미 protocol 3은 그대로입니다. 명시적 Herdr를 포함해 모드 키가 있으면 capability 1이 필수이고 키 없는 기본 Herdr는 legacy 도우미를 허용합니다. **Completed GUI Apply나 신선한 Candidate 인계 거래의 증거는 아닙니다.** 소유 자동화의 `AXIsProcessTrusted=false`였고 네이티브는 UI 승인 없는 수동 계획을 올바르게 거절했습니다.

이전 실제 소유 에이전트 transcript에서 `TERN_REPAIR_SMOKE_OK`를 관측했고 중간 6단계의 `unknown_delivery`는 `TERN_COOPERATIVE_SMOKE_OK` transcript로 조정했으며 맹목 재전송하지 않았습니다. 최종 네 microtask 생산 코드는 reload grace 뒤 현재 live 소스에서 실제 `agents:ask`를 기한 내 수락했습니다: `prompt_accepted`, `committed:false`, `native_applied:false`, `delivery:"accepted_may_be_queued"`. 이후 소유 assistant가 정확히 `TERN_MICROTASK_SMOKE_OK`로 답해 해당 end-to-end 답변을 입증했지만 일반 완료 이벤트나 무기한 신뢰성 보장은 아닙니다. 15초 동안 연결 소스 표본 다섯 번과 실제 정규 A→B 프로필 전환을 관측했고 6.25초 뒤 기존 A는 끊기고 새 B는 연결됐습니다. 무제한 실행 안정성의 증거는 아닙니다.

이전 소스 검사에서 네이티브 테스트 650개(무시 1개)와 실제 AppKit composer 실행 파일, 코디네이터 테스트 67개, TypeScript와 Bun 테스트 58개, arm64 릴리스 앱/도우미 빌드 및 ad-hoc 패키징이 통과했습니다. 격리된 서명 패키지의 Tern 전용 앱은 네이티브 창과 `observation_mode:tern`을 표시했고 `exit_with_herdr:true`여도 91초 넘게 실행되었습니다. **주입한** exchange 목록에서 유니코드/여러 줄 명령이 한 번 제시되었고 주입한 수락 결과는 `prompt_accepted`, `committed:false`, `delivery:"accepted_may_be_queued"`로 기록되었습니다. 목록 만료는 완료 결과 없이 Offline이 되었으며 오래된 워크트리 inspect와 다른 호스트 키의 재사용은 거절되었습니다. 이는 이전의 주입 브리지 관측이며 **실제 Tern 에이전트 전송·완료 증거는 아닙니다**. 이전에 설치된 로그아웃 상태 Tern 0.6.1 창은 별도 플러그인을 열어 `agents:list`/`process.run`을 호출했으나 목록이 비었습니다. 실제 에이전트 목록과 의미 있는 ask는 **그 이전 로그아웃 실행에서만** 미검증이었습니다. 인증 우회는 없었고 이 검사로 GitHub 릴리스·tap·사이트 배포를 수행하지 않았습니다.

추가로 일반 로그아웃 상태 Tern 창과 패키지 바이너리·짧은 격리 XDG 프로필에서 실제 hosts/panes/tabs/sessions/agents(빈 목록) API, 타이머, `process.run`, 전용 교환을 플러그인 오류 없이 실행했습니다. 네이티브 상태는 `observation_mode:tern`, `data_connected:true`, `connected_sources:1`, `sessions:0`, `ui_ready:true`였습니다. 실제 창을 닫은 뒤에도 앱은 실행 중이었고 6초 후에는 `data_connected:false`, `disconnected_sources:1`이었습니다. 주입한 명령 결과 누락은 재제시 없이 `unknown_delivery`가 되었으며 epoch 교체 후 옛 epoch 요청은 `ok:false`로 거절되었습니다. 캡처한 모드로 다시 시작해 새 Tern 인스턴스에 도달했지만 **도우미 주도 전체 업데이트 거래의 완료 증거는 아닙니다**. 위 테스트 수는 메인 649개 통과·1개 무시(메인 총 650개 항목)이며 중첩된 자식 도우미는 별도입니다.

공개 전 이후 검증에서 **이미 실행 중인 현재 사용자 Tern 창**에서 명시적으로 설정한 전용 검증 프로필·플러그인 kv와 Tern CLI로 생성한 본인 소유 일회용 OMP 탭을 사용해, 실제 창 플러그인이 에이전트가 있는 목록을 전용 소켓으로 교환했습니다. 별도의 기존 사용자 pane도 네이티브/플러그인 행에서 확인했습니다. 소유한 pane에 보낸 네이티브 `sessions prompt`는 실제 Tern `agents:ask` 수락 결과 뒤 `prompt_accepted`, `committed:false`, `delivery:"accepted_may_be_queued"`를 반환했습니다. 같은 소유 pane의 Tern `agents:transcript`에는 사용자 요청 `Reply with exactly TERN_PET_SMOKE_OK. Do not use tools or modify any files.`와 assistant 응답 `TERN_PET_SMOKE_OK`가 확인되었고, 에이전트 상태는 idle이며 마지막 메시지도 일치했습니다. 이는 해당 ask와 응답의 증거이지 권위 있는 완료 이벤트나 성공 애니메이션의 증거가 아닙니다. idle만으로 완료를 인증할 수 없습니다. 이전의 새 CLI 창 Keychain 오류는 그 별도 창에 한정되며 현재 사용자 로그인 장애가 아닙니다. 도우미 주도 전체 업데이트 거래·앱 확장·검증 에이전트의 도구/파일 변경·공개 플러그인/아카이브 게시는 없었습니다.

업데이트 프로토콜은 **3** 그대로입니다. 두 업데이트 context 캡처 지점 모두 기본 Herdr이면 `HERDR_PET_OBSERVATION_MODE` 키를 생략하며, 명시적으로 유지된 `herdr` 키는 제거하지 않습니다. 존재하는 키는 정규 모드와 정확한 Original/Candidate 출처 pin을 재시작/복구까지 유지합니다. 앱/도우미는 `{"protocol":3,"observation_mode":1}`을 보고합니다. context 키가 **존재하면** 명시적 `herdr`도 서명된 도우미의 capability `1`이 필요하고, 키 없는 Herdr는 구형 protocol-3 증거를 허용합니다. Tern/both에는 서명된 앱과 도우미의 capability `1` 및 정확히 일치하는 명시적 준비 상태 모드도 필요합니다. 계획/저널 버전은 유지합니다. 불명확한 manager 실행은 재시도할 수 없고 사용자 Stop과 기존 복구 허용 조건이 우선합니다.

[Herdr 호스트 연동 안내](../integrations/herdr/README.md#build-in-separate-checkouts)의 별도 체크아웃 절차를 먼저 따르세요. 공식 Herdr 0.9.3 소스 커밋 `7b116c05bfda646af39d2524c54e70c751f57ee8`에 [client-attach 패치](../integrations/herdr/client-attached.patch)를 적용해 호스트를 빌드하고, 빌드된 실행 파일에 `PATCHED_HERDR="$HERDR_SOURCE/target/release/herdr"`를 지정합니다. 해당 안내의 고정 호스트 도구와 격리된 XDG/세션 설정을 **먼저** 준비하세요. 플러그인 링크는 호스트나 앱을 빌드하지 않습니다. 순정 Herdr 0.9.0과 0.9.3에는 `client.attached` 훅이 없고 `min_herdr_version = "0.9.3"`은 패치 기준일 뿐 훅 지원의 증거가 아닙니다. 실제로 훅 지원을 명시한 향후 호스트 릴리스에서만 패치를 대체할 수 있습니다. 연결 경고·패치된 세션의 API 구독/스키마·안내의 클라이언트 attach probe를 확인하세요. 알 수 없는 훅 경고나 연결 성공만으로는 성공이 아닙니다. `PATH`의 다른 `herdr`가 아닌 동일한 패치 바이너리와 세션을 확인해야 합니다.

연결할 개발 체크아웃은 먼저 직접 빌드하세요(관리형 설치와 달리 `plugin link`는 `[[build]]`를 실행하지 않습니다).

```sh
bash scripts/install.sh --source
"$PATCHED_HERDR" plugin link "$PWD" --enabled
"$PATCHED_HERDR" plugin action invoke start --plugin desktop-pet
```

소스 설치기는 고정된 Bun·npm 의존성을 설치하고 Rust 실행 파일·업데이트 코디네이터를 빌드한 뒤 네이티브 rig 런타임·캐릭터 제작 리소스를 패키징하고 앱을 검증합니다. Rust/Cargo·Bun·Node.js/npm·Xcode Command Line Tools·`codesign`이 필요하며 별도 호스트 빌드 조건은 연동 안내를 따릅니다. 이미 실행 중인 서버에서 연결·활성화만 해도 캐릭터가 시작되는 것은 아닙니다. `start`를 한 번 실행하거나 `auto_start`가 켜졌다면 이후 정상적인 shell/terminal 클라이언트 attach를 기다리세요. 서버 시작 시에도 `ensure`가 자동으로 실행됩니다. 플러그인 액션에는 실행 중인 활성 호스트가 필요하지만 패키지 네이티브 바이너리의 오프라인 설정 명령에는 필요하지 않습니다. 소스 자동화에는 `./dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet`을 사용하세요. 기존 데몬을 종료하거나 `start`·명령·`stop` 모두에 일관된 **설정과 상태** 격리 경로를 사용하세요. 소스 CLI 버전만으로 자산 게시나 구형 실행 데몬의 교체를 증명하지 못하므로 `status`의 실제 실행 파일을 확인하세요.

공통 [설치기](../scripts/install.sh)는 세 모드를 제공합니다. 옵션이 없으면 매니페스트에 고정된 사전 빌드를 시도하고 **없거나 검증 실패한 경우에만** 소스 빌드로 전환합니다. `--prebuilt`는 실패하고 `--source`는 이 체크아웃을 빌드합니다(Bun/npm 의존성은 내려받을 수 있음). 관리형 Herdr 플러그인 설치는 `[[build]]`를 통해 설치기 실행 후 플러그인 디렉터리의 `dist/`에 앱을 둡니다. 매니페스트는 공개 `0.3.6` 사전 빌드를 고정하며 fallback 소스 빌드와 공개 자산 검증은 별개입니다. 성공한 사전 빌드 설치에는 소스 빌드 도구가 필요 없습니다. 설치기는 기존처럼 `HerdrDesktopPet.app/`만 포함한 압축 파일을 받아 체크섬·네이티브 서명을 검증하고 별도 rig 출처 기록을 설치 입력으로 사용하지 않습니다. client-attach 자동 시작에는 여전히 기능이 있는 패치 호스트가 필요합니다.

저장소의 `herdr-plugin` 토픽으로 [Herdr 마켓](https://herdr.dev/plugins/) 자동 수집 대상에 등록됩니다. 인덱스는 30분마다 갱신되며 **심사받지 않은 커뮤니티 목록**이지 호스트 호환성 검토가 아닙니다. Homebrew [개인 tap](https://github.com/hanbong5938/homebrew-tap) 포뮬러는 `HerdrDesktopPet.app`을 `/Applications`가 아닌 Homebrew prefix에 설치하고 CLI를 `PATH`에 놓습니다. Herdr 플러그인이나 패치 호스트는 설치하지 않으므로 실행 관리 훅에는 플러그인, client-attach 자동 실행에는 호스트 패치가 별도로 필요합니다. 포뮬러 로딩이 거부된 경우에만 `brew trust --formula hanbong5938/tap/herdr-desktop-pet`으로 이 포뮬러만 신뢰하세요. 업그레이드 전 데몬을 종료하세요. `start`는 기존 데몬을 재사용하므로 CLI `--version`과 재시작 후 `status`의 `app_version` 및 실행 파일을 함께 확인하세요. 옛 cask는 `/Applications`에 앱을 복사했습니다. `brew uninstall --cask herdr-desktop-pet` 후 `brew install hanbong5938/tap/herdr-desktop-pet`으로 포뮬러에 이전할 수 있으며 사용자 데이터는 유지됩니다. 안정판·베타 프로필 격리와 이전은 [한영 0.3 이전 안내](releases/0.3.md#한국어--v03-계열)를 참고하세요. 소스 버전만으로 공개 자산이 존재한다고 추정하지 마세요.

## 네이티브 빌드와 서명

소스 설치 후 네이티브 검사는 다음 명령으로 실행합니다.

```sh
bun run check
bun run test:native
```

네이티브 hit-overlay 혼합 계산은 릴리스 러너의 Xcode 16.4 Swift 컴파일러와 호환되도록 타입을 지정한 부분 식을 사용합니다. 오버레이를 고칠 때 이 타입 경계를 유지하세요. 소스 빌드에는 여전히 Xcode Command Line Tools가 필요합니다.

debug·release 네이티브 빌드는 `codesign`을 필요로 하며 [`native/rig/RigDecodeWorker.entitlements.plist`](../native/rig/RigDecodeWorker.entitlements.plist)의 `com.apple.security.cs.allow-jit=true`를 **`rig-decode-worker`에만** 부여해 ad-hoc 서명합니다. Cargo는 plist 변경을 추적해 worker를 다시 빌드·서명합니다. JavaScriptCore가 신뢰된 디코더 번들을 JIT 컴파일할 수 있게 하지만 앱 본체와 네이티브 rig 라이브러리에는 **JIT 권한이 없으며** worker 격리·검증·자원 제한은 유지합니다. `bun run package:native`는 선택한 서명 identity로 worker 전용 권한을 다시 적용하고 외부 앱 서명 후 실제 entitlement 및 worker·앱의 엄격한 서명을 검증합니다. 서명·검증 실패 시 **패키징을 중단**합니다. 기본 ad-hoc 패키지는 로컬 실행용이며 Developer ID 서명·공증이 아닙니다.

Rust/Swift rig 경계는 타입 있는 asset/token을 넘기기 **전에** ABI 버전과 C 구조체 일곱 개의 크기를 확인합니다. 초기 motion은 네이티브 호스트가 복사하는 봉인된 바이트이며 옛 경로 조회 방식으로 돌아가지 않습니다. Metal은 같은 인코더의 색상 그리기 전에 실제 흰자 구멍을 L/R/무방향 stencil로 준비하고 CPU 의미론적 hit·참조 래스터도 같은 소유권 규칙을 적용합니다. 흰자가 없는 rig의 기존 홍채 표시 동작은 유지합니다. `RIG_PROBE_FAULTS` probe의 `--eye-stencil-checks --output ABS`는 실제 픽셀·의미론적 fixture를 저장하지만 과거 취소/연결 해제 상황의 시각 현상 원인을 입증하지 않습니다.

`scripts/build-rig-native.mjs`는 범위 제한된 소스/컴파일러/출력 해시를 `native/target/rig-native/release/rig-native.json`에 기록합니다. 패키징은 소스·생성 출력의 해시를 확인하고 원본 기록을 `Contents/Resources/rig-native-build.json`에 포함하며 독립 업데이트 코디네이터와 protocol/서명 검사도 유지합니다. 앱 서명 **이후** 코디네이터를 포함한 `Contents`의 모든 배포 파일 해시를 외부 `dist/HerdrDesktopPet.app.rig-native.json`에 기록해 서명 번들 안의 자체 해시 순환을 피합니다. 릴리스 워크플로는 기존 앱 전용 압축 파일·체크섬 두 파일 외에 별도 `HerdrDesktopPet-v0.3.6-rig-native.json` 자산을 게시합니다. 설치기/업데이터의 아카이브 입력 형식은 바꾸지 않으며 실제 추출된 서명 후 파일과 사이드카의 해시를 비교해야 배포 바이트 출처를 주장할 수 있습니다.

## 저장소 구조

| 경로 | 역할 |
| --- | --- |
| [`herdr-plugin.toml`](../herdr-plugin.toml) | Herdr 플러그인 실행 관리·액션 정의 |
| [`native/`](../native/) | Rust 앱·네이티브 UI·세션 처리·캐릭터 관리 |
| [`native/rig/`](../native/rig/), [`web/rig/`](../web/rig/) | Rig 렌더링·디코딩·제작 지원 |
| [`assets/`](../assets/) | 기본 루벨리아 팩·메뉴 썸네일만 번들 |
| [`tools/`](../tools/) | 범용 캐릭터 제작·rig 검사 도구 |
| [`scripts/`](../scripts/) | 설치·네이티브 빌드·앱 패키징 |
| [`.github/workflows/release.yml`](../.github/workflows/release.yml) | 릴리스 패키징 워크플로 |

## 네이티브 현지화

네이티브 UI 문구는 정적 카탈로그 [`native/src/i18n.rs`](../native/src/i18n.rs)에 있으며 중국어 간체와 일본어 카탈로그는 각각 [`native/src/i18n/zh_hans.rs`](../native/src/i18n/zh_hans.rs), [`native/src/i18n/ja.rs`](../native/src/i18n/ja.rs)입니다. UI 메시지를 추가할 때는 모든 카탈로그를 갱신하고 런타임 값·경로·ID·팩 소유 메타데이터는 번역하지 마세요. 환경 설정의 정규 저장 토큰은 `system|ko|en|zh-Hans|ja`, 대화문 override 토큰은 `ko|en|zh-Hans|ja`이며 언어별 초안은 독립적입니다. CLI/패치 검사는 엄격하게 유지하고 기존 저장 설정의 알 수 없는 값에는 관대한 fallback을 유지하세요. `system`은 OS 선호 언어를 순서대로 검사합니다. 일본어 태그는 대소문자와 무관하며 중국어 `zh`·명시적 Hans·CN/SG 지역은 간체로 해석합니다. 명시적 Hant 또는 Hans가 없는 TW/HK/MO는 다음 지원 언어로 넘기고 없으면 영어를 사용합니다. 외부 카탈로그 메타데이터는 `ko`에 한국어, `en`·`zh-Hans`·`ja`에 영어 fallback을 사용하고 작성된 대화문과 fallback은 팩 소유를 유지합니다.

네이티브 문구나 CLI 언어 계약을 바꿀 때 [CLI 계약](cli.md), README [English](../readme.md)·[한국어](../readme.ko.md)·[简体中文](../readme.zh-Hans.md)·[日本語](../readme.ja.md), 사용 안내 [English](usage.md)·[한국어](usage.ko.md)·[简体中文](usage.zh-Hans.md)·[日本語](usage.ja.md)를 함께 갱신하세요. 이 문서들은 네이티브 앱에 관한 것이며 홍보 웹사이트의 네 언어 번역은 별도로 관리하세요.

## 캐릭터 제작

팩 제작·검증은 [캐릭터 제작 가이드](../.agents/skills/character-creator/SKILL.md)와 [`tools/character-pack.py`](../tools/character-pack.py)를 참고하세요. 배포 앱에도 `Contents/Resources/creator/SKILL.md`·`Contents/Resources/creator/character-pack.py`가 포함됩니다. 소스 체크아웃의 [로컬 Qwen + See-through 제작 스킬](../.agents/skills/create-pet-character/SKILL.md)은 **승인된 그림에서** 독립된 10포즈를 만듭니다. 외부 엔진·모델 가중치는 번들에 포함하지 않습니다. 루벨리아가 기본 rig 템플릿입니다. PNG 템플릿·Aurora·Coding Cat·선택형 루벨리아 의상과 각각의 그림 생성기는 별도 [herdr-characters](https://github.com/hanbong5938/herdr-characters)에서 관리합니다. 외부 템플릿은 `--templates-root PATH`로 지정해야 하며 번들에 있다고 가정하지 마세요.

가져온 팩의 라이선스·출처·사용 조건 파일을 함께 보관하세요. 기본 루벨리아의 [라이선스](../assets/rubelia-default/LICENSE.txt)·[출처](../assets/rubelia-default/ATTRIBUTION.txt)·[제작 기록](../assets/rubelia-default/source-record.json)·[배경 보정 기록](../assets/rubelia-default/background-repair.json)이 그림 기준입니다. [`LICENSE.txt`](../LICENSE.txt)는 원래 Coding Cat 고지를 유지하지만 Coding Cat과 그 그림 생성 소스는 별도 캐릭터 저장소로 이동했습니다. 이 고지들이 저장소의 모든 구성 요소에 일괄 라이선스를 부여하지는 않습니다. 그림 소유자의 승인은 모델 권리를 부여하지 않으며 소프트웨어 MIT는 그림 라이선스를 바꾸지 않습니다. Qwen 연구 라이선스는 제작 기록으로 보존하고 모델 가중치·엔진은 번들에 넣지 않습니다. 버전별 다시 그리기·얼굴 보정·당시 한계는 [v0.1.6](releases/v0.1.6.md)·[v0.1.9](releases/v0.1.9.md)·[v0.1.10](releases/v0.1.10.md)을 참고하세요.

## 웹사이트 (GitHub Pages)

현재 사이트의 제품 시각 자료는 정적 이미지이며 로컬 Pages 작업에 동영상·MP4 자산이 필요하지 않습니다.

캡처를 게시하기 전에 화면 내용을 직접 확인하세요. 격리된 시연 프로필에서 앱만 담고 실제 세션 내용·계정명·로컬 경로·호스트명·이메일·인증 정보를 포함하지 마세요. 공개 PNG에는 이미지·색상 청크만 허용하며 `check`와 `stage`는 EXIF·텍스트·프로필 등 메타데이터를 거부합니다. 이 검사는 화면에 보이는 글자를 판별하지 않으므로 개인정보 수동 검토도 반드시 필요합니다.

한국어 기본 홍보 웹사이트는 [`site/`](../site/)에 있습니다. 네이티브 macOS 앱을 설명하는 사이트이며 브라우저에서 앱을 실행하지 않습니다. Pages 주소: [한국어](https://hanbong5938.github.io/herdr-desktop-pet/) · [영어](https://hanbong5938.github.io/herdr-desktop-pet/en/) · [중국어 간체](https://hanbong5938.github.io/herdr-desktop-pet/zh-Hans/) · [일본어](https://hanbong5938.github.io/herdr-desktop-pet/ja/). 제품 문구를 바꿀 때 네 페이지의 번역과 언어 전환 링크를 함께 갱신하고 `ko`·`en`·`zh-Hans`·`ja`의 상호 `hreflang` 및 한국어 루트를 가리키는 `x-default`를 유지하세요. 각 페이지의 canonical URL과 Open Graph 언어도 해당 페이지에 맞춰야 합니다. 외부 [캐릭터 갤러리](https://github.com/hanbong5938/herdr-characters)는 별도로 유지되며 한국어·영어만 제공합니다. 과거 v0.3.2 스크린샷을 최신 화면이나 번역된 스크린샷처럼 소개하지 마세요. 영어 소셜 미리보기 이미지를 공유해도 페이지 언어는 바뀌지 않습니다. 제목은 화면 폭에 맞춘 간결한 크기와 좁은 주변 여백을 사용하며 본문·터미널 명령어·버튼 크기는 제목과 독립적으로 유지합니다. 저장소 루트에서 실행하세요.

```sh
bun test tools/pages.test.ts
node tools/pages.mjs check
node tools/pages.mjs serve --port 4173
# http://127.0.0.1:4173/herdr-desktop-pet/ 접속 (영어: /herdr-desktop-pet/en/; 중국어 간체: /herdr-desktop-pet/zh-Hans/; 일본어: /herdr-desktop-pet/ja/)
node tools/pages.mjs stage /tmp/herdr-pages
```

사이트 CLI에는 **Node.js 22**를 사용하고 **Bun 1.4.2**는 실제 Node 서버를 실행하는 HTTP 회귀 테스트의 러너로만 사용합니다. CI는 staging 전에 이 회귀 테스트를 실행합니다. `check`는 내부 링크·앵커·언어별 메타데이터·에셋을 확인하며 외부 링크를 네트워크로 검사하지 않습니다. 로컬 미리보기는 공개 파일 중 일반 파일만 제공합니다. 최종 `404.html` 대체 파일이 없거나 심볼릭 링크·일반 파일이 아니라면 파일을 전송하는 대신 `nosniff`가 있는 일반 텍스트 `500`을 반환하며 `HEAD`에는 본문이 없습니다. `stage`는 먼저 검사한 뒤 저장소 **밖의 새 디렉터리 또는 빈 디렉터리**에 공개할 `site/` 파일만 복사하며 배포하지 않습니다. Pages 워크플로는 PR을 검사하고 검토용 아티팩트만 첨부하며 배포하지 않습니다. Pages 배포 소스는 **GitHub Actions**이고, 웹사이트 변경을 `main`에 병합하거나 `main`에서 워크플로를 수동 실행했을 때 조건을 만족하는 `main` 실행만 배포합니다. 이 로컬 명령은 릴리스 바이너리·Wiki·저장소 Pages 설정을 바꾸지 않습니다.
