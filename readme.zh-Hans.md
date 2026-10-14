# Desktop Pet — Herdr integration

[English](readme.md) · [한국어](readme.ko.md) · [日本語](readme.ja.md) · [简体中文](readme.zh-Hans.md)

**当前公开分发：**[Core v0.4.5](https://github.com/hanbong5938/desktop-pet/releases/tag/v0.4.5)，Release `412667385`，`2026-10-14T12:01:32Z`，准确共同源码 `51dcf22415b1ff0faf07651cb83c1c696c980f43`，protocol4/observation2/API1。当前 consumer 认证实际公开 installer 字节。明确选择 legacy ingress 才使用真正 [bridge v0.3.10](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/v0.3.10)，Release `412684474`，`2026-10-14T12:20:45Z`，metadata 源码 `67eb44b53bda6ba96eaf68671b67a94cab7b304d`：同一 Core 构建的真实 legacy3/1/API1，Git 仅11个 regular UTF-8 metadata 文件与限定生成 app ignore。两个公开资产集合、严格签名和68-file closure 均已验证。旧 stable formula `7bf1fd7f1b89bd77e68179fcee95cacf044927dc` 选择 v0.3.10。Herdr **v0.4.1** 保持不变的 `6d2ac0446f6ecb84f179e21fac3c5b02e07dc8e8` / 无资产 Release `412252863`，Tern v0.2.0 与兼容 Core pin 不变。不创建新集成标签，不由 Tern 安装/自动启动 native，也不替换旧标签/正文/资产/beta。

**当前 Core 的实际运行证据：**fresh public Brew v0.4.5 安装与只读检查没有启动 daemon、创建 standalone app 或中立 profile。明确 Start 后实际 GUI 显示 Desktop Pet / Auto / Herdr Waiting0 / Tern Waiting0；PID `1971` / instance `27752246b4da5c6f3964a74dd22881a2` 在明确 Hide 后仍以同一实例运行。另一既有 profile 的普通 v0.4.4→v0.4.5 更新 `a6991e69233377c32b150186c62888fd` 经真实 Brew PID=PGID `9853`、exit0/drained true 完成。更新共享 owner 前只明确 Stop 自有无关默认 profile，没有暗中重启或合并。新 PID `10377` / instance `37953e017fc9208bfcc785a4ff138494` 在准确旧 profile、Herdr1/Tern0 下 UI/control ready，后续实际 Check 为绿色正常结果。旧证据/profile 除 daemon log 外不变，未重装 Herdr。被阻塞的旧 v0.4.3 checker 没有自行更新；经过签名原本备份、明确 Stop、真实固定 installer、明确 Start 的同 owner 修复后，v0.4.5 Check 也通过。旧公开 v0.3.9/v0.4.4 A/B、87个证据和无关默认 profile 保持证据仍在[原版本账本](https://github.com/hanbong5938/desktop-pet/blob/main/docs/releases/v0.4.4.md)，不替代新组合验收。

**新公开 managed A→B 已完成：**真实 native 冻结3.6→公开3.10 A `20cca91548584437af112bb04cc260b0` 经 terminal Check、独立 Move preview 后，另行 protocol4 同意 B `6ee850aa5577314c6d9cf6ebc7eae981` 完成到既有 Brew4.5 PID3594 / instance2ee0995b8b3dc3abe18e6238a91929d9，准确旧 config/state、保存 Herdr/H1T0、真实 UI/control ready/Commit。真实 Herdr PID=PGID3648/exit0/quiescent=true 刷新到准确6d2，Git clean 且 legacy app 已移除。原 A87文件和四个 profile/evidence tree 的全部既有文件字节不变，原签名3.6 helper 仍报告 completed；无关 default1971 无 Stop/合并，保持同实例 Hidden 运行。[当前账本](https://github.com/hanbong5938/desktop-pet/blob/main/docs/releases/v0.4.5.md)。

**保留历史失败：**真实 Brew v0.3.6 → v0.3.7 → Core v0.4.2 A→B 成功与 managed v0.3.7 A 的逻辑 `/tmp` 缺陷是独立结果。后者保持 **Unknown**，不恢复、重放或 reset。原公开 v0.3.8 A 成功，但 B preflight 因准确68个生成 app untracked 文件而在 B 同意与 Stop 前拒绝。没有 `.gitignore` 的原10文件 metadata 标签不被追认为成功。公开 v0.3.9 仅增加准确 `/dist/HerdrDesktopPet.app/` 所有权声明。真实 CI native `/tmp` 与生成 app Git gate 已通过；tracked script 修改、无关 root file 与 `dist/user-owned-note.txt` 仍为 dirty。手工 ignore、user file clean/reset 或放宽 dirty 保护不是迁移步骤。

**独立验证与限制：**实际公开3.9 beta 路径拒绝 live 替换及 protocol2 直接 bridge，安装原冻结3.6 matched pair 而不自动启动；同一 beta config/state 的明确 Start/真实 ready、原签名 helper 与用户源码/profile 保持均已验证，之后仅 Stop 自有3.6。不声称执行过新3.10 beta 入口。f7edf5e 的确定 retirement 失败及新同意 same-plan resume 是独立历史证据。Pages 和真实 SDK 双顺序/prompt/transcript/reconnect 必须独立验证；正常 Tern keychain、OMP provider SignIn/NoModel 不可用，不伪造 account/ACK/transcript。

仅安装需要的集成。Herdr 是轻量消费者；Tern 不下载、构建、安装或自动启动原生应用。插件更新与卸载只影响集成，不删除共享应用、配置档或角色。常规应用更新不重新安装 Herdr 插件。

[Desktop Pet / core app](https://github.com/hanbong5938/desktop-pet/blob/main/readme.zh-Hans.md) · [Herdr host / patch / OMP / link](integrations/herdr/README.md) · [Legacy A → B migration](https://github.com/hanbong5938/desktop-pet/blob/main/docs/migrations/desktop-pet.md)

[Core 应用网站](https://hanbong5938.github.io/desktop-pet/zh-Hans/) · [Core Wiki](https://github.com/hanbong5938/desktop-pet/wiki) · [Herdr 集成网站](https://hanbong5938.github.io/herdr-desktop-pet/zh-Hans/)。旧网站仅提供集成与迁移入口，不复制应用 UI 或美术。

Core v0.4.5 CI `38560075408` release job `115838673595` 与真正 bridge CI `38561115955` job `115842136732` 均通过，包含实际 native 逻辑 `/tmp` 和生成 app Git 保护。后续 Wiki job `115840956329` 渲染成功，但因缺少专用 `WIKI_PUBLISH_TOKEN` 发布失败。仍需所有者初始化 Home 与专用凭据，不使用广泛 `gh`/`GITHUB_TOKEN` fallback。当前 Core/Herdr Pages 不能仅凭 source/link 声称部署成功。

公开中立 formula 选择 Core v0.4.5 ([tap `d66411383141644f01f1ca839086b6d505a997ac`](https://github.com/hanbong5938/homebrew-tap/commit/d66411383141644f01f1ca839086b6d505a997ac))。仅安装共享 app 与需要的 main 集成：

```sh
brew install hanbong5938/tap/desktop-pet
herdr plugin install hanbong5938/herdr-desktop-pet
herdr plugin action invoke start --plugin desktop-pet
```

Apple Silicon Mac、macOS 13 或更新版本。源码构建需 Rust/Cargo、Bun、Node.js/npm、Xcode Command Line Tools，预构建无需这些工具。应用为 ad-hoc 签名，未经公证。Herdr 快照需兼容本地服务器（0.9.0+），回复另需 `agent.prompt`。可选 client-attach 自动启动需官方 Herdr 0.9.3 应用[集成仓库的补丁和宿主构建指引](https://github.com/hanbong5938/herdr-desktop-pet/blob/main/integrations/herdr/README.md)，或实际支持钩子的宿主。原版 0.9.0/0.9.3 没有该钩子，插件安装不修改宿主。

预构建与源码安装在 **core** 检出目录中执行，而不是 Herdr 集成目录。默认仅预构建，下载或验证失败不会暗中回退源码编译。`--source` 明确仅构建当前 core，并使用该目录的 `dist/DesktopPet.app`。

## Herdr consumer 约定

本 repository 不构建或复制 `DesktopPet.app`、native package 或 character asset。集成 manifest/wrapper/adapter 属于此 checkout，native source/art/tool 属于 Core。当前 `core-release.json` / `release-evidence/` 从真实公开 Core v0.4.5 Release `412667385`、准确 `51dcf22415b1ff0faf07651cb83c1c696c980f43` materialize。installer SHA-256 为 `fdcd9d80f8672b54ceca967bfea6f72223d2e5caed97f845140a68b9e7414a16`，checksum asset 为 `8c9c8ab020bc4561c4a2c17f4c6d20b3e8a18672b84f508426e68ed259598c5e`。[实际权威与运行证据](https://github.com/hanbong5938/desktop-pet/blob/main/docs/releases/v0.4.5.md)不修改或重新发布不变 v0.4.1 集成标签的历史 Core v0.4.2 最低 pin。

准确发现兼容共享应用的顺序为：显式绝对可执行路径 → 已验证的真实账户主目录共享 locator → PATH 中原生 `desktop-pet` → 实际 Brew prefix → 真实账户主目录的 `Applications/DesktopPet.app`。无效显式路径或 locator 不会回退至其他安装。仅应用缺失时允许通过固定且 digest 已验证的 **core** installer bootstrap；不兼容安装需显式原生升级或修复，不在此构建产品或静默 fallback。

Herdr 配置档优先级为注入的 `HERDR_PLUGIN_CONFIG_DIR` / `HERDR_PLUGIN_STATE_DIR` → 显式 `--config-dir` / `--state-dir` → Herdr XDG/HOME 默认值。wrapper 将两个解析后路径显式传入 core，不移动或合并配置档。后端来源通过 `--herdr-socket ABS` 传入，不是旧 `--socket` 或 pet 的私有控制套接字。`status` 显示实际运行映像及配置档，而不只是 CLI。

[Herdr 指南](integrations/herdr/README.md) 保留现有 21 个 action ID 及其角色显示、气泡、生命周期语义，以及宿主构建、补丁、link 和 OMP 指引。`ensure` 幂等且不会显示隐藏的 pet。可选 attach 自动启动需真正支持钩子的宿主。lifecycle 文件原先不存在时 Herdr 退出策略默认为 true；已有文件缺少该键时为 false。写入保留显式策略与未知字段。Tern 是独立 consumer，不自动启动共享应用。

## 源码 / link

先在 **desktop-pet Core** checkout 执行 `bash scripts/install.sh --source` 安装或构建 app，再 link **此集成 checkout** 而非 Core。`plugin link` 不构建。当前 consumer 固定真实 Core v0.4.5，已实际验证同一4.5 instance 的 Hide→ensure 保持。历史 v0.4.1 / Core v0.4.2 并发 bootstrap/wrapper 证据单独保留，不创建新集成标签。参阅[独立 checkout 流程](integrations/herdr/README.md#build-in-separate-checkouts)；plugin 安装不 patch Herdr。

通用[使用指南](https://github.com/hanbong5938/desktop-pet/blob/main/docs/usage.zh-Hans.md)、[CLI](https://github.com/hanbong5938/desktop-pet/blob/main/docs/cli.md)、[开发](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.md)及原生功能、provider 详情均属于独立 core。

## 所有权

本仓库拥有 adapter、manifest、轻量 wrapper、Herdr/OMP 集成、集成验证及冻结历史。应用、UI、rig、美术、包装、源码构建属于 [desktop-pet](https://github.com/hanbong5938/desktop-pet)。`.herdrchar`、pack v5、`default@0`、rig ABI 及 [herdr-characters](https://github.com/hanbong5938/herdr-characters) 权威不变。公开正文、标签、素材保持不变。修正 v0.4.1 已通过整个 filesystem/index/archived-tree 所有权检查，不包含原生 bundle/archive/rig 产品。

## 美术与许可

[软件许可](LICENSE.txt)与美术／模型条款**彼此独立**：MIT 软件许可并不重新授权角色美术。内置露贝莉娅的[许可](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/LICENSE.txt)、[署名](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/ATTRIBUTION.txt)、[来源记录](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/source-record.json)及[背景修复记录](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/background-repair.json)具有权威性。`LICENSE.txt` 还保留原 Coding Cat 声明；其美术／源码已移至[角色仓库](https://github.com/hanbong5938/herdr-characters)。请保留每个可选角色包自己的条款。美术所有者的批准不授予模型权利；Qwen 的研究许可仅为制作记录，应用未附带模型引擎／权重。参阅[角色制作与许可（英语）](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.md#character-authoring)。
