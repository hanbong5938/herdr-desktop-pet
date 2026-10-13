# Herdr Desktop Pet

[Herdr](https://herdr.dev) 的原生 macOS 桌面伙伴。露贝莉娅会回应触摸，在桌面显示观察到的代理会话，并让你向选中的本地会话发送回复。

[English](readme.md) · [한국어](readme.ko.md) · **简体中文** · [日本語](readme.ja.md) · [版本文档（英语）](docs/releases/README.md) · [发布版本](https://github.com/hanbong5938/herdr-desktop-pet/releases)

<img src="assets/rubelia-thumbnail.png" alt="默认桌面伙伴露贝莉娅" width="220">

[浏览可选角色与服装图库](https://hanbong5938.github.io/herdr-characters/)（网站默认英语，可选择 KO 查看韩语）· [角色包源码与许可](https://github.com/hanbong5938/herdr-characters)。**仅露贝莉娅（`default@0`）随应用附带。**

[v0.3.6](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/v0.3.6) 是已公开的最新稳定版，公开 arm64 资产、校验和与更新的稳定 Homebrew 配方已验证。匹配的应用／辅助程序报告 `protocol: 3` 和 `observation_mode: 1`。历史 v0.3.5 和 beta 资产保持不变。

## 功能

- 可移动、调整大小的桌面角色，支持触摸与抚摸反馈、独立状态气泡和菜单栏控制；界面可跟随系统语言，或选择韩语、英语、简体中文、日语。
- 观察到的 Herdr 会话卡片支持搜索、筛选与排序，并明确区分本地／远程来源；远程观察可选且只读。
- 向选中的**本地**代理会话发送单行回复，支持输入法组合输入与原生状态反馈。
- 可配置角色、气泡、对话和可见性；支持可选的已验证角色包与官方目录浏览器。
- Herdr 生命周期操作、受保护的关联工作树移除，以及明确区分安装来源的应用更新。

## 系统要求

- **Apple Silicon Mac，macOS 13 或更新版本。** Intel Mac、Windows、Linux 均不属于支持的原生运行平台。
- 本地观察使用现有本地 Herdr 服务器（会话快照需 0.9.0 或更新版本），无需插件或主程序补丁；回复另外要求服务器支持 `agent.prompt`。**仅可选的客户端连接自动启动**需要官方 Herdr 0.9.3 源码应用所提供的 [`client.attached` 补丁](integrations/herdr/client-attached.patch)后构建的主程序，或未来确实声明支持该钩子的正式版本。原版 Herdr 0.9.0 和 0.9.3 都没有此钩子；清单中的最低版本要求不提供此功能。管理式安装请遵循[主程序集成指南（英语）](integrations/herdr/README.md)，不要忽略未知钩子警告。
- 源码构建需要 Rust/Cargo、Bun、Node.js/npm 和 Xcode Command Line Tools；预构建版本不需要源码工具链。应用采用 **ad-hoc 签名，未使用 Developer ID 签名，也未经公证**；macOS Gatekeeper 可能需要你明确地在本机批准。签名验证不代表已经公证。

## 使用 Homebrew 安装：首次连接

使用下面的稳定配方安装公开预构建包，无需 Rust/Cargo、Bun、Node.js 或源码检出。已有 Brew 用户请使用下方升级命令。现有配置档与偏好设置会保留。

```sh
brew install hanbong5938/tap/herdr-desktop-pet
herdr-desktop-pet --version
herdr-desktop-pet start
herdr-desktop-pet status
```

原生 `start` 是手动启动，不要求 `herdr plugin install`、`plugin link` 或打过补丁的主程序。展开气泡可查看该服务器的本地会话卡片（快照需要 Herdr 0.9.0+）；实时卡片需要活跃的代理会话。发送回复还要求服务器支持 `agent.prompt`；提交 ACK 并不表示代理已完成。没有卡片时，检查实际运行的 Herdr CLI／服务器及 `HERDR_SOCKET_PATH` / `HERDR_CLIENT_SOCKET_PATH`。如果后端套接字与角色所使用的环境变量／默认值不同，请使用**该服务器实际打印的套接字路径**，不要猜测：

```sh
herdr-desktop-pet --socket /actual/path/to/herdr-api.sock start
herdr-desktop-pet status
```

将示例路径替换为已核实的 Herdr **后端**套接字；`--socket` 不是角色私有的控制套接字。`start` 会复用已运行的守护进程，因此更改后端前先停止旧进程；通过 `status` 核对实际守护进程的版本和可执行文件，不要只看 `--version`。参阅[连接故障排查](docs/usage.zh-Hans.md#配置与问题排查)。

升级现有安装前，**先停止旧守护进程**，否则 `start` 会继续复用它：

```sh
herdr-desktop-pet stop
brew update && brew upgrade herdr-desktop-pet
herdr-desktop-pet --version
herdr-desktop-pet start
herdr-desktop-pet status
```

确认 CLI 报告 `0.3.6`，并且重启后 `status` 同时显示 `app_version: 0.3.6` **和预期的运行中可执行文件**。配方把应用安装在 Homebrew 前缀中，**不是** `/Applications`，并把 CLI 放在 `PATH` 上；它既不安装 Herdr 插件，也不提供打过补丁的主程序。旧守护进程必须先停止。有关配置隔离和旧 cask 迁移，参阅 [0.3 升级说明（英语）](docs/releases/0.3.md#upgrade-from-v021-or-beta)。

### 不连接 Herdr 的独立角色

若没有健康的本地 Herdr 服务器，却希望角色持续显示，请在启动**之前**关闭新配置默认启用的 30 秒断线退出：

```sh
herdr-desktop-pet settings set exit_with_herdr off
herdr-desktop-pet settings get
herdr-desktop-pet start
herdr-desktop-pet status
```

无需安装 Herdr、插件或补丁。启动后可用 `herdr-desktop-pet hide_bubble` 隐藏空气泡，同时保留角色。若约 30 秒后角色消失，请检查 `settings get` 和 `status`：在 `exit_with_herdr` 开启且服务器不健康时，宽限期后会退出。请在重新启动前、针对同一实际配置／状态配置文件将其关闭。该开关**不能**覆盖所有已注册本地插件端点均已确认禁用／解除关联所导致的退出。参阅[生命周期说明](docs/usage.zh-Hans.md#生命周期与插件操作)。

### 可选进阶：管理式 Herdr 插件和连接自动启动

在目标兼容 Herdr 主程序上安装[插件仓库](https://github.com/hanbong5938/herdr-desktop-pet)，然后明确启动角色：

```sh
herdr plugin install hanbong5938/herdr-desktop-pet
herdr plugin action invoke start --plugin desktop-pet
```

管理式安装运行 `bash scripts/install.sh`：先尝试清单指定的 v0.3.6 仅含 `.app` 的压缩包，并验证校验和、归档条目、arm64 签名及内置角色。使用**已公开 v0.3.6 Release 的完整压缩包／校验和及匹配安装器**；单独上传的来源证明附属文件不在安装器归档中。预构建资产缺失或无效时，默认安装器会报告问题并从当前检出源码构建；显式 `--prebuilt` 则会失败，不会回退；`--source` 构建当前检出。安装插件不会为 Herdr 打补丁或构建 Herdr。要使用连接自动启动，请依照[指南（英语）](integrations/herdr/README.md#build-in-separate-checkouts)准备打过补丁的主程序；上述手动原生观察均不需要它。

## 从此检出源码安装，供管理式集成使用

**仅针对可选的管理式连接工作流**：先在本仓库根目录按[主程序构建流程（英语）](integrations/herdr/README.md#build-in-separate-checkouts)，在独立检出中给官方 Herdr 0.9.3 打补丁并构建，运行其隔离服务器，再把 `PATCHED_HERDR` 设为**构建好的主程序可执行文件**（例如 `PATCHED_HERDR="$HERDR_SOURCE/target/release/herdr"`）。以下命令应使用同一目标主程序／配置／套接字；`PATH` 上未经补丁的二进制文件无法提供连接自动启动。仅从源码构建角色不需要打过补丁的主程序。

```sh
bash scripts/install.sh --source
"$PATCHED_HERDR" plugin link "$PWD" --enabled
"$PATCHED_HERDR" plugin action invoke start --plugin desktop-pet
```

`plugin link` **不会**执行清单构建；请先构建角色。在已经运行的主程序上关联／启用也不会立即启动角色：手动调用一次 `start`，或在启用 `auto_start` 后等待下一次符合条件的 shell／终端客户端连接。检查关联警告：未知的 `client.attached` 钩子表示主程序缺少所需补丁。Herdr 操作要求主程序已运行且已启用；服务器启动时也会自动执行 `ensure`。
清单中的生命周期与操作命令使用 `scripts/run-native.sh`：稳定清单版本选择 `HerdrDesktopPet.app`，`-beta.N` 选择 `HerdrDesktopPetBeta.app`；缺少对应包时会失败，不会改用另一个渠道。直接从源码自动化可使用 `./dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet`；先停止旧守护进程，或同时隔离**配置和状态**路径，然后用 `status` 核对实际可执行文件／版本。参阅[源码开发（英语）](docs/development.md#source-development)。


## Tern 安装与独立配置档

先安装原生应用，再显式安装独立的公开插件。插件不会下载、构建或启动应用。

```sh
brew install hanbong5938/tap/herdr-desktop-pet
tern plugin install https://github.com/hanbong5938/tern-desktop-pet
PET="$(command -v herdr-desktop-pet)"
PET_CONFIG="$HOME/.config/herdr/plugins/config/desktop-pet-tern"
PET_STATE="$HOME/.local/state/herdr/plugins/desktop-pet-tern"
"$PET" --host tern --config-dir "$PET_CONFIG" --state-dir "$PET_STATE" stop
"$PET" --version
"$PET" --host tern --config-dir "$PET_CONFIG" --state-dir "$PET_STATE" start
"$PET" --host tern --config-dir "$PET_CONFIG" --state-dir "$PET_STATE" status
```

默认宿主仍为 Herdr。`--host tern` 无需 Herdr 服务器、套接字或插件。在实际 Tern 窗口中用 **Configure desktop pet connection** 设置绝对可执行文件 `PET` 以及上述配置和状态两个路径，再用 **Connect to desktop pet** 连接。输入实际路径，不是变量名。检查窗口连接和同一配置档的原生 `status`；发现插件不等于已连接。更换程序或模式前先停止同一配置档。源码开发仍为可选：在当前检出运行 `bash scripts/install.sh --source`，设置 `PET="$PWD/dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet"`，无需特殊分支。应用／辅助程序必须匹配 protocol 3，附加 JSON 能力为 `protocol: 3` 和 `observation_mode: 1`。英语、韩语、简体中文、日语均保留。**接收测试版更新**默认关闭，需主动选择；检查不会安装或重启。

## 首次使用

### 应用内更新（此检出采用协议 3）

**设置 → 应用更新**分别提供默认开启的**自动检查**、默认关闭的**接收测试版更新**、**立即检查**、运行版本／来源、按策略记录的上次成功检查时间，以及明确的操作按钮。只检查稳定版时会选取官方稳定发行版；选择接收测试版后按语义版本选取符合条件的最高稳定版或测试版（相同基础版本的正式版高于测试版）。关闭测试版选项不会降级已经安装的测试版。应用打开期间自动检查至多每天一次，稳定版和包含测试版的策略分别记录时间。切换测试版策略会使旧候选失效；仅在自动检查开启时按新策略检查，否则请使用**立即检查**。检查是只读的：开关和检查均不会安装、重启、刷新 Brew 元数据或修改 Git。跨发布版本／渠道／源码与目标的变更，都需要针对具体提案重新获得原生界面的单独同意。

卡片区分当前检查／交接活动和保留的**上次操作**结果；**显示诊断信息**可展开可选中的路径、哈希、ID 及原始错误。简短状态及辅助程序提示会跟随选择的 UI 语言。磁盘上已安装候选不等于已经应用或正在运行；源码为当前版本或固定版本的结果也不能证明它是最新公开发行版。Herdr 管理式迁移会在同一兼容的运行中主程序上将官方发行标签固定至不可变提交，不会重置检出。Homebrew 保留捕获的管理器／前缀并明确指定目标配方／版本；经确认的操作会重新验证元数据并预留两个安装根目录，不会暗中卸载原配方。本地检出仅重建磁盘上的文件，不执行 Git pull/reset。仅经过验证、兼容且已安装的软件包，才可能无需重新执行安装器而提供**应用已安装更新**；未知来源不会被隐式转换为管理式安装。

原生同意与 Prepare 会在冻结会修改状态的控件前，重新检查待处理编辑、输入法组合输入和交互；原进程在交接期间继续运行。所有权 ACK 既不是安装完成，也不是允许 Stop：只有接受 Prepare 后，匹配的最终可见／已准备 ACK 才转移同意；只有完整送达且被接受的已预留 Stop 才提交更新器关机。暂存工作在工作线程执行，辅助窗口保持响应；超过原生界面 30 秒交接期限意味着**等待，而不是取消**。辅助程序检查替换进程就绪情况，分别记录已安装和已应用。候选启动使用日志／预留记录中准确、已验证的**映像与安装来源**组合，绝不从当前磁盘或原始源码令牌推断来源。不确定或旧协议的预留会保留证据并继续阻止操作；恢复不会重放安装器、悄悄删除日志，也不会自动重启原进程。用户主动 Stop 可阻止复活，但已证实安全停止的事务仍可完成对账并释放自身预留。不确定操作重试前，请参阅[更新行为与恢复](docs/usage.zh-Hans.md#应用内更新)及 [CLI 状态与恢复（英语）](docs/cli.md#app-update-status-and-recovery)。

已发布的稳定版 `v0.3.3` 和配套 `0.3.3-beta.1` 归档仍是不可变的、应用与辅助程序协议 2 配套包：仅检查更新**不会**使它们获得此检出中的协议 3 测试版设置。此检出的源码版本或原生 CLI 版本并不能单独证明协议 3 发行包已发布。使用该协议需要明确从此协议 3 检出构建，或另行验证已发布的协议 3 软件包。应用／辅助程序的协议配对不可混用；旧协议 1／2 记录需要对应的保留辅助程序，而不是由新更新器转换。公开版本请核实实际[发布资产](https://github.com/hanbong5938/herdr-desktop-pet/releases)。

## 操作

| 操作 | 结果 |
| --- | --- |
| 轻点头部／身体；在头部滑动 | 触发反应；抚摸露贝莉娅。 |
| 拖动角色；使用右下角把手 | 移动角色；调整角色大小。 |
| 拖动气泡背景或边缘／角落 | 移动所附角色或独立气泡；独立调整气泡大小。 |
| 左键点击菜单栏图标 | 打开角色、观察、气泡及生命周期设置。 |
| 展开气泡，选择本地代理卡片 | 打开单行回复；Enter、Command+Enter 或发送按钮提交。远程卡片只读。 |

输入法组合输入期间**绝不提交**；Escape 首先取消组合输入。在搜索框按 Enter／Command+Enter 不会发送回复。角色与气泡的可见性互相独立；两者都隐藏或启用整窗点击穿透后，可能无法通过桌面点击找回。必要时使用菜单栏图标的**显示并启用角色**恢复操作（默认模式始终显示图标；仅恢复模式在必要时显示）。完整的操作、搜索、通过已保存机器进行远程观察和可见性行为，参阅[使用指南](docs/usage.zh-Hans.md#操作)。

## 可选角色和服装

[角色图库](https://hanbong5938.github.io/herdr-characters/)展示可选角色包；[独立仓库](https://github.com/hanbong5938/herdr-characters)保存源码与使用条款。在设置中选择**角色 → 浏览角色…**；浏览肖像不会下载归档。符合条件的已发布官方角色包提供明确、经过验证的**下载并应用**。对于本地文件夹或 `.herdrchar`，仅**导入**不会激活：还需选择已安装卡片／修订版本，再点击**应用**。只有露贝莉娅随应用附带且为默认角色。参阅[角色包](docs/usage.zh-Hans.md#角色包)及[制作说明（英语）](docs/development.md#character-authoring)。

## 安全与限制

- 回复只调用选中的**本地** Herdr 来源的 `agent.prompt`，不使用 SSH、shell 或向窗格直接输入；远程卡片不能发送。Herdr 的 ACK 表示已提交，**不表示代理已完成**。请在实际终端处理批准／问题；手动重发前先核查不确定的送达状态。提交前后若会话／窗格被替换，存在当前占用者竞态，并无原子性的会话保证。参阅[送达详情](docs/usage.zh-Hans.md#内联回复与消息传递)。
- **移除工作树…**不可撤销：继续前确认**准确的检出路径**和**整个工作区**的所有标签页／窗格。该操作会终止其中的进程／代理并移除关联检出，**包括被忽略的文件**；不会进入废纸篓，无法撤销。主仓库和分支不会删除。参阅[移除防护措施](docs/usage.zh-Hans.md#删除工作树)。
- 远程观察需要选定已保存的机器配置及兼容的远程 Herdr 服务器，**不要求**在远端安装桌面角色。本地应用轮询远端状态，无法恢复短暂遗漏的状态转换，也不能发送远程提示。参阅[观察指南](docs/usage.zh-Hans.md#观察来源)。
- 自动检查应用更新是只读的；安装或应用更新需要明确同意。更新器所有权 ACK **不是完成信号**。结果不明时，重试前请查阅[应用内更新恢复](docs/usage.zh-Hans.md#应用内更新)及 [CLI 更新状态与恢复（英语）](docs/cli.md#app-update-status-and-recovery)。

## 配置与故障排查

`HERDR_PLUGIN_CONFIG_DIR` 保存 `preferences.json`、`lifecycle.json`、管理的 `characters/` 和 `menu-bar-icons/`；`HERDR_PLUGIN_STATE_DIR` 保存锁、控制套接字及 `desktop-pet.log`。未注入路径时，使用 Herdr 的 XDG 插件路径；原生 `--config-dir` / `--state-dir` 可隔离配置文件，但注入的环境变量路径优先。升级或切换渠道前，备份**两个实际目录**。`lifecycle.json` 单独控制 `auto_start` 和 `exit_with_herdr`，不属于其他偏好设置。

如果无服务器启动约 30 秒后角色消失，使用 `settings get` 检查 `exit_with_herdr`，并在独立使用的重启**之前**关闭它；已注册本地端点全部被确认禁用／解除关联仍会终止本次运行。如果角色隐藏，请通过上述菜单栏操作恢复，不必重新安装。如果角色没有在管理式主程序上出现，请检查其 `client.attached` 警告、手动 `start`／`status`、实际可执行文件／版本和守护进程日志；关闭自动启动可能导致有意跳过启动。连接错误请检查 `HERDR_SOCKET_PATH` / `HERDR_CLIENT_SOCKET_PATH` 及实际持有后端套接字的服务器；远程错误请检查已保存的配置和本地 CLI。请在**希望显示角色的 Mac**上运行本应用。提供商集成属于上游 Herdr；安装其钩子后重新加载现有提供商会话。参阅[故障排查](docs/usage.zh-Hans.md#配置与问题排查)与[生命周期](docs/usage.zh-Hans.md#生命周期与插件操作)。

## 文档与网站

- [使用与恢复](docs/usage.zh-Hans.md) · [开发与安装（英语）](docs/development.md) · [原生自动化 CLI 契约（英语）](docs/cli.md#english) · [打补丁的主程序集成（英语）](integrations/herdr/README.md)。
- [发行历史（英语）](docs/releases/README.md) · [v0.3.6 变更（英语／韩语）](docs/releases/v0.3.6.md) · [v0.3 升级指南（英语）](docs/releases/0.3.md#upgrade-from-v021-or-beta) · [历史测试版／预览版指南（英语）](docs/migrations/unreleased.md) · [v0.1.11 → v0.2.0 迁移（英语）](docs/migrations/v0.1.11-to-v0.2.0.md)。
- 宣传网站提供[简体中文](https://hanbong5938.github.io/herdr-desktop-pet/zh-Hans/)、[韩语](https://hanbong5938.github.io/herdr-desktop-pet/)、[英语](https://hanbong5938.github.io/herdr-desktop-pet/en/)和[日语](https://hanbong5938.github.io/herdr-desktop-pet/ja/)版本，介绍的是此**原生**应用；应用不能在浏览器中运行。网站开发／部署参阅 [GitHub Pages 指南（英语）](docs/development.md#website-github-pages)。独立的[角色图库](https://hanbong5938.github.io/herdr-characters/)展示可选角色包，仅提供英语和韩语。

## 美术与许可

[软件许可](LICENSE.txt)与美术／模型条款**彼此独立**：MIT 软件许可并不重新授权角色美术。内置露贝莉娅的[许可](assets/rubelia-default/LICENSE.txt)、[署名](assets/rubelia-default/ATTRIBUTION.txt)、[来源记录](assets/rubelia-default/source-record.json)及[背景修复记录](assets/rubelia-default/background-repair.json)具有权威性。`LICENSE.txt` 还保留原 Coding Cat 声明；其美术／源码已移至[角色仓库](https://github.com/hanbong5938/herdr-characters)。请保留每个可选角色包自己的条款。美术所有者的批准不授予模型权利；Qwen 的研究许可仅为制作记录，应用未附带模型引擎／权重。参阅[角色制作与许可（英语）](docs/development.md#character-authoring)。
