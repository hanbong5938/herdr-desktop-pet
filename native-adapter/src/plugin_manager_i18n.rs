#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UiLocale {
    En,
    Ko,
    ZhHans,
    Ja,
}
impl UiLocale {
    pub(crate) fn from_tag(tag: &str) -> Self {
        match tag {
            "ko" => Self::Ko,
            "zh-Hans" => Self::ZhHans,
            "ja" => Self::Ja,
            _ => Self::En,
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) enum Message {
    PluginManagerTitle,
    PluginManagerHelp,
    PluginManagerOpen,
    PluginManagerInstall,
    PluginManagerRemove,
    PluginManagerSaving,
    PluginManagerAvailable,
    PluginManagerUnavailable,
    PluginManagerAvailabilityUncertain,
    PluginManagerRemoveConfirmation,
    PluginManagerSaveFailure,
    Remove,
    Cancel,
}
pub(crate) fn text(locale: UiLocale, message: Message) -> &'static str {
    match locale {
        UiLocale::Ko => match message {
            Message::PluginManagerTitle => "플러그인 관리자",
            Message::PluginManagerHelp => "이 펫에서 필요할 때 플러그인 관리자를 여세요. 기능 설치·제거는 플러그인에 영향을 주지 않습니다.",
            Message::PluginManagerOpen => "관리자 열기",
            Message::PluginManagerInstall => "기능 설치",
            Message::PluginManagerRemove => "기능 제거",
            Message::PluginManagerSaving => "관리자 기능 상태 확인 중…",
            Message::PluginManagerAvailable => "관리자 기능이 설치되었습니다. 열 때만 플러그인 목록을 불러옵니다.",
            Message::PluginManagerUnavailable => "관리자 기능을 설치하여 사용하세요. 플러그인은 변경되지 않습니다.",
            Message::PluginManagerAvailabilityUncertain => "설정 요청의 응답을 확인할 수 없습니다. 현재 저장 상태를 확인한 뒤 계속하세요.",
            Message::PluginManagerRemoveConfirmation => "플러그인 관리자 기능만 제거합니다. Herdr 플러그인과 파일, 설정 및 상태는 그대로 유지됩니다. 계속할까요?",
            Message::PluginManagerSaveFailure => "플러그인 관리자 기능 설정을 저장할 수 없습니다",
            Message::Remove => "제거",
            Message::Cancel => "취소",
        },
        UiLocale::En => match message {
            Message::PluginManagerTitle => "Plugin manager",
            Message::PluginManagerHelp => "Installing or removing this feature does not change plugins.",
            Message::PluginManagerOpen => "Open",
            Message::PluginManagerInstall => "Install",
            Message::PluginManagerRemove => "Remove",
            Message::PluginManagerSaving => "Checking manager availability…",
            Message::PluginManagerAvailable => "Manager feature installed. Plugins are loaded only when opened.",
            Message::PluginManagerUnavailable => "Install the manager feature to use it. Plugins are unchanged.",
            Message::PluginManagerAvailabilityUncertain => "Preference acknowledgement is uncertain. Check the current saved state before continuing.",
            Message::PluginManagerRemoveConfirmation => "Remove only the plugin manager feature. Herdr plugins, files, configuration and state remain unchanged. Continue?",
            Message::PluginManagerSaveFailure => "Plugin manager availability could not be saved",
            Message::Remove => "Remove",
            Message::Cancel => "Cancel",
        },
        UiLocale::ZhHans => match message {
            Message::PluginManagerTitle => "插件管理器",
            Message::PluginManagerHelp => "需要时可从此宠物打开插件管理器。安装或移除此功能不会影响插件。",
            Message::PluginManagerOpen => "打开管理器",
            Message::PluginManagerInstall => "安装功能",
            Message::PluginManagerRemove => "移除功能",
            Message::PluginManagerSaving => "正在检查管理器功能状态…",
            Message::PluginManagerAvailable => "管理器功能已安装。仅在打开时加载插件列表。",
            Message::PluginManagerUnavailable => "请安装管理器功能后使用。插件不会发生变化。",
            Message::PluginManagerAvailabilityUncertain => "无法确认设置请求的响应。请先检查当前已保存的状态，再继续操作。",
            Message::PluginManagerRemoveConfirmation => "仅移除插件管理器功能。Herdr 插件、文件、配置和状态均保持不变。是否继续？",
            Message::PluginManagerSaveFailure => "无法保存插件管理器功能设置",
            Message::Remove => "移除",
            Message::Cancel => "取消",
        },
        UiLocale::Ja => match message {
            Message::PluginManagerTitle => "プラグインマネージャー",
            Message::PluginManagerHelp => "このペットで必要なときにプラグインマネージャーを開いてください。この機能のインストールや削除はプラグインに影響しません。",
            Message::PluginManagerOpen => "マネージャーを開く",
            Message::PluginManagerInstall => "機能をインストール",
            Message::PluginManagerRemove => "機能を削除",
            Message::PluginManagerSaving => "マネージャー機能の状態を確認中…",
            Message::PluginManagerAvailable => "マネージャー機能はインストール済みです。プラグイン一覧は開いたときだけ読み込みます。",
            Message::PluginManagerUnavailable => "マネージャー機能をインストールしてご利用ください。プラグインは変更されません。",
            Message::PluginManagerAvailabilityUncertain => "設定リクエストへの応答を確認できません。現在の保存状態を確認してから続行してください。",
            Message::PluginManagerRemoveConfirmation => "プラグインマネージャー機能だけを削除します。Herdr のプラグイン、ファイル、設定、状態はそのまま保持されます。続行しますか？",
            Message::PluginManagerSaveFailure => "プラグインマネージャー機能の設定を保存できませんでした",
            Message::Remove => "削除",
            Message::Cancel => "キャンセル",
        },
    }
}
