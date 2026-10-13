//! Herdr-owned connection copy, kept out of the host-neutral UI.
#[derive(Clone, Copy)]
pub(crate) enum Message {
    Exit, ExitHelp, Local, LocalHelp, Remote, RemoteHelp, Loading, Empty,
    NoSources, NotSelected, Paused, Disabled, Connecting, Online, Offline,
    CopyReconnect, ReconnectHelp,
}

pub(crate) fn text(locale: &str, message: Message) -> &'static str {
    use Message::*;
    if locale.starts_with("ko") {
        match message {
            Exit => "Herdr와 함께 종료",
            ExitHelp => "Herdr만 선택했을 때 연결 해제 30초 후 종료합니다. Auto에서는 다른 호스트와 pet을 유지합니다.",
            Local => "로컬 Herdr 세션",
            LocalHelp => "이 Mac의 로컬 Herdr 세션을 관찰합니다.",
            Remote => "저장된 Herdr 머신",
            RemoteHelp => "터미널에서 herdr machine add <SSH-host>로 머신을 등록한 뒤 선택하세요. 저장된 세션을 5초마다 읽기 전용으로 확인하며 메시지는 보내지 않습니다. SSH 인증과 재연결은 터미널에서 직접 진행하세요.",
            Loading => "저장된 머신을 읽는 중…",
            Empty => "저장된 머신이 없습니다. 터미널에서 머신을 등록한 뒤 선택하세요.",
            NoSources => "Herdr 관찰 대상이 꺼져 있습니다. 이 Mac 또는 원격 머신을 켜세요.",
            NotSelected => "선택되지 않음",
            Paused => "관찰 꺼짐",
            Disabled => "Herdr에서 비활성화됨",
            Connecting => "상태 확인 중…",
            Online => "관찰 중",
            Offline => "상태를 읽지 못함",
            CopyReconnect => "재연결 명령 복사",
            ReconnectHelp => "저장된 Herdr 머신에 다시 연결하려면 이 명령을 터미널에서 실행하세요.",
        }
    } else if locale.starts_with("ja") {
        match message {
            Exit => "Herdrと一緒に終了",
            ExitHelp => "Herdrのみ選択中は切断から30秒後に終了します。Autoではほかのホストとペットを維持します。",
            Local => "ローカルHerdrセッション",
            LocalHelp => "このMac上のローカルHerdrセッションを観察します。",
            Remote => "保存済みHerdrマシン",
            RemoteHelp => "ターミナルでherdr machine add <SSH-host>を実行してマシンを登録し、選択してください。保存済みのセッションを5秒ごとに読み取り専用で確認し、メッセージは送りません。SSH認証と再接続はターミナルで行ってください。",
            Loading => "保存済みのマシンを読み込み中…",
            Empty => "保存済みのマシンがありません。ターミナルで登録してから選択してください。",
            NoSources => "Herdrの観察元がオフです。このMacまたはリモートマシンをオンにしてください。",
            NotSelected => "未選択",
            Paused => "観察オフ",
            Disabled => "Herdrで無効",
            Connecting => "状態を確認中…",
            Online => "観察中",
            Offline => "状態を取得できません",
            CopyReconnect => "再接続コマンドをコピー",
            ReconnectHelp => "保存済みHerdrマシンに再接続するには、このコマンドをターミナルで実行してください。",
        }
    } else if locale.starts_with("zh") {
        match message {
            Exit => "随Herdr退出",
            ExitHelp => "仅选择Herdr时，断开连接30秒后退出。Auto会保留其他主机和桌面宠物。",
            Local => "本地Herdr会话",
            LocalHelp => "观察这台 Mac 上的本地 Herdr 会话。",
            Remote => "已保存的Herdr设备",
            RemoteHelp => "在终端中运行herdr machine add <SSH-host>注册设备，然后选择它。每5秒只读检查已保存的会话，不会发送消息。SSH身份验证和重新连接请在终端中完成。",
            Loading => "正在加载已保存的设备…",
            Empty => "没有已保存的设备。请先在终端中注册设备，然后选择它。",
            NoSources => "Herdr 观察来源已关闭。请开启“这台 Mac”或“远程设备”。",
            NotSelected => "未选择",
            Paused => "观察已关闭",
            Disabled => "已在 Herdr 中禁用",
            Connecting => "正在检查状态…",
            Online => "观察中",
            Offline => "无法获取状态",
            CopyReconnect => "复制重新连接命令",
            ReconnectHelp => "请在终端中运行此命令，重新连接已保存的Herdr设备。",
        }
    } else {
        match message {
            Exit => "Exit with Herdr",
            ExitHelp => "Exit after 30 seconds without Herdr only when Herdr is selected. Auto keeps the pet and other hosts alive.",
            Local => "Local Herdr sessions",
            LocalHelp => "Observe local Herdr sessions on this Mac.",
            Remote => "Saved Herdr machines",
            RemoteHelp => "Register a machine with herdr machine add <SSH-host> in a terminal, then select it. Saved sessions are checked every 5 seconds, read-only; no messages are sent. Complete SSH authentication and reconnect in your terminal.",
            Loading => "Loading saved machines…",
            Empty => "No saved machines. Register one in your terminal, then select it.",
            NoSources => "Herdr observation sources are off. Enable this Mac or a remote machine.",
            NotSelected => "Not selected",
            Paused => "Observation off",
            Disabled => "Disabled in Herdr",
            Connecting => "Checking status…",
            Online => "Observing",
            Offline => "Status unavailable",
            CopyReconnect => "Copy reconnect command",
            ReconnectHelp => "Run this command in your terminal to reconnect the saved Herdr machine.",
        }
    }
}
