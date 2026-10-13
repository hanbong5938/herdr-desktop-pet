# Herdr 연동 사용 안내

[English](usage.md) · [한국어](usage.ko.md) · [日本語](usage.ja.md) · [简体中文](usage.zh-Hans.md)

현재 네이티브 앱 조작·캐릭터·프로필·제공자 연결·복구는 독립 Desktop Pet core 소유입니다.

- [현재 앱 사용법](https://github.com/hanbong5938/desktop-pet/blob/main/docs/usage.ko.md) · [앱 사이트](https://hanbong5938.github.io/desktop-pet/)
- [Core CLI](https://github.com/hanbong5938/desktop-pet/blob/main/docs/cli.md#한국어) · [Core Wiki](https://github.com/hanbong5938/desktop-pet/wiki)
- [Herdr action·훅·실행 정책·호스트 패치·link·OMP](../integrations/herdr/README.md)
- [기존 A → B 이전](https://github.com/hanbong5938/desktop-pet/blob/main/docs/migrations/desktop-pet.md)
- [Tern Connect·Configure](https://github.com/hanbong5938/tern-desktop-pet/blob/main/README.ko.md)

앱은 core installer로 별도 설치하거나 실제 포뮬러 게시 후 `brew install hanbong5938/tap/desktop-pet`을 사용하세요. 선택형 연동은 `herdr plugin install hanbong5938/herdr-desktop-pet`와 `tern plugin install https://github.com/hanbong5938/tern-desktop-pet`입니다. Herdr는 공용 앱을 소비하고 Tern은 앱을 설치·자동 시작하지 않습니다. 실제 config/state 두 경로를 선택하며 프로필을 이동·병합하지 않습니다. 플러그인 제거는 앱·프로필·캐릭터를 남깁니다.

[게시 경계](../readme.ko.md): 링크나 명령은 실제 릴리스·포뮬러·Pages·Wiki 공개를 입증하지 않습니다. 실제 Tern/OMP 수락을 주장하지 않습니다. 동결된 [v0.3.6 문서](https://github.com/hanbong5938/herdr-desktop-pet/tree/v0.3.6/docs)와 [과거 릴리스](releases/README.ko.md)는 역사적 기록이며 현재 설치 지침이 아닙니다.
