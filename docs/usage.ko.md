# Herdr 연동 사용 안내

[English](usage.md) · [한국어](usage.ko.md) · [日本語](usage.ja.md) · [简体中文](usage.zh-Hans.md)

현재 네이티브 앱 조작·캐릭터·프로필·제공자 연결·복구는 독립 Desktop Pet core 소유입니다.

- [현재 앱 사용법](https://github.com/hanbong5938/desktop-pet/blob/main/docs/usage.ko.md) · [앱 사이트](https://hanbong5938.github.io/desktop-pet/)
- [Core CLI](https://github.com/hanbong5938/desktop-pet/blob/main/docs/cli.md#한국어) · [Core Wiki](https://github.com/hanbong5938/desktop-pet/wiki)
- [Herdr action·훅·실행 정책·호스트 패치·link·OMP](../integrations/herdr/README.md)
- [기존 A → B 이전](https://github.com/hanbong5938/desktop-pet/blob/main/docs/migrations/desktop-pet.md)
- [Tern Connect·Configure](https://github.com/hanbong5938/tern-desktop-pet/blob/main/README.ko.md)

앱은 core installer로 별도 설치하거나 실제 포뮬러 게시 후 `brew install hanbong5938/tap/desktop-pet`을 사용하세요. 선택형 연동은 `herdr plugin install hanbong5938/herdr-desktop-pet`와 `tern plugin install https://github.com/hanbong5938/tern-desktop-pet`입니다. Herdr는 공용 앱을 소비하고 Tern은 앱을 설치·자동 시작하지 않습니다. 실제 config/state 두 경로를 선택하며 프로필을 이동·병합하지 않습니다. 플러그인 제거는 앱·프로필·캐릭터를 남깁니다.

Herdr가 선택되어 있으면 설정에서 선택형 내장 플러그인 관리자의 **열기 / 설치 / 제거**를 제공합니다. 설치와 제거는 이 프로필의 기능 사용 여부만 저장하며 Herdr 플러그인을 설치하거나 삭제하지 않습니다. 기능 제거에는 확인이 필요합니다. 명시적으로 열기를 선택한 뒤에만 실제 선택된 Herdr 소켓과 인증된 호스트 실행 파일로 목록을 읽습니다. 플러그인 변경은 별도의 메타데이터 재확인과 승인을 거치며 보호된 펫 설치는 읽기 전용입니다. 창을 닫아도 이미 전달된 작업은 취소되지 않습니다. 결과가 불명확하면 기록을 검토해야 하며 자동 재시도하지 않습니다. 설정 응답이 유실되어도 원래 작업 ID를 유지하고 변경 요청을 재전송하지 않은 채 저장 상태를 다시 확인합니다.

[게시 경계](../readme.ko.md): 링크나 명령은 실제 릴리스·포뮬러·Pages·Wiki 공개를 입증하지 않습니다. 실제 Tern/OMP 수락을 주장하지 않습니다. 동결된 [v0.3.6 문서](https://github.com/hanbong5938/herdr-desktop-pet/tree/v0.3.6/docs)와 [과거 릴리스](releases/README.ko.md)는 역사적 기록이며 현재 설치 지침이 아닙니다.
