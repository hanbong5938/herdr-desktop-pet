# Desktop Pet — Herdr integration

[English](readme.md) · [한국어](readme.ko.md) · [日本語](readme.ja.md) · [简体中文](readme.zh-Hans.md)

**当前公开分发：**[Core v0.4.8](https://github.com/hanbong5938/desktop-pet/releases/tag/v0.4.8)，Release `413822006`，发布时间 `2026-10-15T13:06:02Z`，准确不可变源码 `2936b2c600235ec8d15d1248ef3380e0a8dca8c0`，neutral protocol4/observation2/API1。真正 [bridge v0.3.12](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/v0.3.12)，Release `413831645`，发布时间 `2026-10-15T13:15:47Z`，ingress `f05a95a46dee042acf613c8d126d25be597fc824`，annotated tag object `eb7656fa6465148110b021b2d28efc3dcaa428a5`，由同一 Core 源码构建 genuine legacy3/1/API1。Core 的6个匿名公开资产、arm64 system/helper closure、strict deep 签名、68个部署资源已独立验证；bridge 的5资产与 CI 字节完全一致，公开11文件 Core producer closure、strict deep 签名、68资源、paired3/1/API1 也通过。公开 [tap `ef95ec0be315f3601f18849dce67a11bd706af26`](https://github.com/hanbong5938/homebrew-tap/commit/ef95ec0be315f3601f18849dce67a11bd706af26) 选择 stable Core4.8/legacy3.12；beta 不变。Herdr **v0.4.1** `6d2ac0446f6ecb84f179e21fac3c5b02e07dc8e8` / 无资产 Release `412252863` 仍仅是 Herdr-managed source 的冻结 retirement authority。Tern0.2.0/兼容 pin 不变，无新集成标签，也不在此 consumer 构建 native。

## 历史运行证据 — 不是 Core4.8/bridge3.12 验收

以下4.5/3.10及更早结果保留原版本、身份与证据。下文“当前”“新”仅指当时执行，不代表今天的公开分发或实际用户 Mac 状态。

**当前 Core 的实际运行证据：**fresh public Brew v0.4.5 安装与只读检查没有启动 daemon、创建 standalone app 或中立 profile。明确 Start 后实际 GUI 显示 Desktop Pet / Auto / Herdr Waiting0 / Tern Waiting0；PID `1971` / instance `27752246b4da5c6f3964a74dd22881a2` 在明确 Hide 后仍以同一实例运行。另一既有 profile 的普通 v0.4.4→v0.4.5 更新 `a6991e69233377c32b150186c62888fd` 经真实 Brew PID=PGID `9853`、exit0/drained true 完成。更新共享 owner 前只明确 Stop 自有无关默认 profile，没有暗中重启或合并。新 PID `10377` / instance `37953e017fc9208bfcc785a4ff138494` 在准确旧 profile、Herdr1/Tern0 下 UI/control ready，后续实际 Check 为绿色正常结果。旧证据/profile 除 daemon log 外不变，未重装 Herdr。被阻塞的旧 v0.4.3 checker 没有自行更新；经过签名原本备份、明确 Stop、真实固定 installer、明确 Start 的同 owner 修复后，v0.4.5 Check 也通过。旧公开 v0.3.9/v0.4.4 A/B、87个证据和无关默认 profile 保持证据仍在[原版本账本](https://github.com/hanbong5938/desktop-pet/blob/main/docs/releases/v0.4.4.md)，不替代新组合验收。

**新公开 managed A→B 已完成：**真实 native 冻结3.6→公开3.10 A `20cca91548584437af112bb04cc260b0` 经 terminal Check、独立 Move preview 后，另行 protocol4 同意 B `6ee850aa5577314c6d9cf6ebc7eae981` 完成到既有 Brew4.5 PID3594 / instance2ee0995b8b3dc3abe18e6238a91929d9，准确旧 config/state、保存 Herdr/H1T0、真实 UI/control ready/Commit。真实 Herdr PID=PGID3648/exit0/quiescent=true 刷新到准确6d2，Git clean 且 legacy app 已移除。原 A87文件和四个 profile/evidence tree 的全部既有文件字节不变，原签名3.6 helper 仍报告 completed；无关 default1971 无 Stop/合并，保持同实例 Hidden 运行。[当前账本](https://github.com/hanbong5938/desktop-pet/blob/main/docs/releases/v0.4.5.md)。

**保留历史失败：**真实 Brew v0.3.6 → v0.3.7 → Core v0.4.2 A→B 成功与 managed v0.3.7 A 的逻辑 `/tmp` 缺陷是独立结果。后者保持 **Unknown**，不恢复、重放或 reset。原公开 v0.3.8 A 成功，但 B preflight 因准确68个生成 app untracked 文件而在 B 同意与 Stop 前拒绝。没有 `.gitignore` 的原10文件 metadata 标签不被追认为成功。公开 v0.3.9 仅增加准确 `/dist/HerdrDesktopPet.app/` 所有权声明。真实 CI native `/tmp` 与生成 app Git gate 已通过；tracked script 修改、无关 root file 与 `dist/user-owned-note.txt` 仍为 dirty。手工 ignore、user file clean/reset 或放宽 dirty 保护不是迁移步骤。

**独立验证与限制：**实际公开3.9 beta 路径拒绝 live 替换及 protocol2 直接 bridge，安装原冻结3.6 matched pair 而不自动启动；同一 beta config/state 的明确 Start/真实 ready、原签名 helper 与用户源码/profile 保持均已验证，之后仅 Stop 自有3.6。不声称执行过新3.10 beta 入口。f7edf5e 的确定 retirement 失败及新同意 same-plan resume 是独立历史证据。Pages 和真实 SDK 双顺序/prompt/transcript/reconnect 必须独立验证；正常 Tern keychain、OMP provider SignIn/NoModel 不可用，不伪造 account/ACK/transcript。

## 当前路径与验收边界

**历史 Core4.7 VM 路径：**公开 Brew3.10 先通过普通 **A′更新至 genuine3.11**，确认原 helper terminal/quiescent settlement，再进行独立 Move preview、Source Review、另行最终 native 同意 **B至 Core4.7**，完成 readiness/Commit 后由所属 manager retirement。Brew 仅卸载批准的 stable legacy formula；冻结集成4.1仅供 Herdr-managed source。冻结3.6兼容路径与 beta→原冻结3.6 matched pair 路径保持独立。不得 replay/reset Unknown 或替换 helper/journal。

[历史 Core4.7 ledger](https://github.com/hanbong5938/desktop-pet/blob/main/docs/releases/v0.4.7.md)记录真实公开 Brew3.10→普通3.11 A′·原 helper 结清→独立同意公开 B4.7→readiness/Commit·真实所属 Brew retirement，并保留原字节、导入角色/台词/历史及无关 profile。当时 consumer 复用、最终 native Check 与 Cancel/rollback/Stop/Unknown/确定失败/native Resume 均已实际验证。不将这些结果改称 Core4.8/bridge3.12 验收。

[当前 Core4.8 正本 ledger](https://github.com/hanbong5938/desktop-pet/blob/main/docs/releases/v0.4.8.md)区分修正后的公开字节、当前 consumer metadata 与实际宿主验收。用户 Mac 已通过真实所属 Brew 完成 bridge3.11→3.12 更新，exit0/`NO_CLEANUP`；安装 app/helper 的 strict deep 签名及准确源码 `2936b2c600235ec8d15d1248ef3380e0a8dca8c0` / legacy3.12 / 3/1/API1 identity 均通过。原 profile pair 已 Start，实际 instance `3edfc569f4d793c7d2f2cb1d71cd4487` 达到 HerdrH1T0/UI/control ready，Settings 已打开。此前3.11因不支持 v2 history 而阻塞的画面保留为历史重现，不证明新3.12仍阻塞或已经解除阻塞。新 native Check/解除阻塞仍**等待人工 TCC 批准，尚未观测**；独立最终 B、Core4.8 installed 验收、retirement、最终 native Check 同样未观测。历史 terminal plan/profile 只读验证保留原字节，当前执行/恢复仍拒绝旧 v2 history；实际 profile throwaway smoke 已通过。ready3.12、公开资产和 metadata 验证不能代替 native 同意或新 B 验收。H1/T0 表示 Tern 未配置，不是真实 Tern-account/双顺序/provider 证明。

仅安装需要的集成。Herdr 是轻量消费者；Tern 不下载、构建、安装或自动启动原生应用。插件更新与卸载只影响集成，不删除共享应用、配置档或角色。常规应用更新不重新安装 Herdr 插件。

[Desktop Pet / core app](https://github.com/hanbong5938/desktop-pet/blob/main/readme.zh-Hans.md) · [Herdr host / patch / OMP / link](integrations/herdr/README.md) · [Legacy A → B migration](https://github.com/hanbong5938/desktop-pet/blob/main/docs/migrations/desktop-pet.md)

[Core 应用网站](https://hanbong5938.github.io/desktop-pet/zh-Hans/) · [Core Wiki](https://github.com/hanbong5938/desktop-pet/wiki) · [Herdr 集成网站](https://hanbong5938.github.io/herdr-desktop-pet/zh-Hans/)。旧网站仅提供集成与迁移入口，不复制应用 UI 或美术。

Core release workflow `38739112250`、binary job `116435899893` 与 genuine bridge CI `38740106737`、job `116439269337` 均为绿色通过。Wiki job `116438922256` 仅缺专用 `WIKI_PUBLISH_TOKEN`，不使用广泛 credential 替代或重新运行 binary。本次 main/default branch 与4语言 Pages 更新尚未部署；公开 release 可用不等于 Pages 部署证明。

公开 formula 选择 Core4.8 ([tap `ef95ec0`](https://github.com/hanbong5938/homebrew-tap/commit/ef95ec0be315f3601f18849dce67a11bd706af26))。此前 [tap `6693cd9`](https://github.com/hanbong5938/homebrew-tap/commit/6693cd9db26e6dba9a257bbe76acaf78cb502c2f)保留为历史 Core4.7/legacy3.11 证据。仅从审核后的 default branch 安装共享 app 及所需集成：

```sh
brew install hanbong5938/tap/desktop-pet
herdr plugin install hanbong5938/herdr-desktop-pet
herdr plugin action invoke start --plugin desktop-pet
```

Apple Silicon Mac、macOS 13 或更新版本。源码构建需 Rust/Cargo、Bun、Node.js/npm、Xcode Command Line Tools，预构建无需这些工具。应用为 ad-hoc 签名，未经公证。Herdr 快照需兼容本地服务器（0.9.0+），回复另需 `agent.prompt`。可选 client-attach 自动启动需官方 Herdr 0.9.3 应用[集成仓库的补丁和宿主构建指引](https://github.com/hanbong5938/herdr-desktop-pet/blob/main/integrations/herdr/README.md)，或实际支持钩子的宿主。原版 0.9.0/0.9.3 没有该钩子，插件安装不修改宿主。

预构建与源码安装在固定到公开 Core4.8 源码 `2936b2c600235ec8d15d1248ef3380e0a8dca8c0` 的 **core** 检出目录中执行，而不是 Herdr 集成目录。默认仅预构建，下载或验证失败不会暗中回退源码编译。`--source` 明确仅构建该 core，并使用该目录的 `dist/DesktopPet.app`。

## Herdr consumer 约定

本 repository 是不构建或复制 `DesktopPet.app`、native package、character asset 的轻量 consumer。当前 metadata/evidence8文件已从匿名公开 Core4.8 Release `413822006`、源码 `2936b2c600235ec8d15d1248ef3380e0a8dca8c0` 实际 materialize。当前 `core-release.json` 的 installer SHA-256 为 `bf329d3b7a1571129d48bb809f967cc6ad503738cb601a1e592666b9ca3737d8`；checksum asset 为 `428d81f39a5d78b9da46af519afbc1e7a2a1ca7274c25fab16418cff08574fbf`；release API record 为 `be089c3c9bf5d7d5ff8735efa131b8d1f8da62ef47cfe16b29b6a0b22868a7f0`。这些真实文件已通过既有 materializer、adapter104测试和 TypeScript/shell gate。这是 materialize 证据，不是新 Core4.8 安装或 native 批准。manifest 集成4.1/API1、managed retirement `6d2ac0446f6ecb84f179e21fac3c5b02e07dc8e8`、历史 Core4.2最低 pin 均不变，无新集成标签。

**历史 Core4.7 consumer 证据：**metadata/evidence8文件从匿名公开 Release `413584934`、源码 `52514da5115f09aac22de5429c08f7acdd993258` 实际 materialize。installer SHA-256 `f64a27593058a68c5a5e112f448be644c6abfbb760be109acf992e2f415f5829`；checksum asset `2619293634c27cfc6cbb7db11c9dd8407ac719b3e2c4321d6a6dc5cf21937d5e`；release record `0643094bc94adad0ad3a18120a3a69c2688900889ee9f34befe8f94e31186923`。adapter104测试通过，实际 `install.sh` / `run-native.sh status` 复用同一 Standalone4.7 PID1446 / instance `d10dcb4df0f131506f0b38a0a9f7aedd`、显式 profile、Herdr1/Tern0，无安装/启动/Herdr重装。[历史账本](https://github.com/hanbong5938/desktop-pet/blob/main/docs/releases/v0.4.7.md)不重新发布或更改冻结集成4.1及历史 Core4.2最低 pin。

准确发现兼容共享应用的顺序为：显式绝对可执行路径 → 已验证的真实账户主目录共享 locator → PATH 中原生 `desktop-pet` → 实际 Brew prefix → 真实账户主目录的 `Applications/DesktopPet.app`。无效显式路径或 locator 不会回退至其他安装。仅应用缺失时允许通过固定且 digest 已验证的 **core** installer bootstrap；不兼容安装需显式原生升级或修复，不在此构建产品或静默 fallback。

Herdr 配置档优先级为注入的 `HERDR_PLUGIN_CONFIG_DIR` / `HERDR_PLUGIN_STATE_DIR` → 显式 `--config-dir` / `--state-dir` → Herdr XDG/HOME 默认值。wrapper 将两个解析后路径显式传入 core，不移动或合并配置档。后端来源通过 `--herdr-socket ABS` 传入，不是旧 `--socket` 或 pet 的私有控制套接字。`status` 显示实际运行映像及配置档，而不只是 CLI。

[Herdr 指南](integrations/herdr/README.md) 保留现有 21 个 action ID 及其角色显示、气泡、生命周期语义，以及宿主构建、补丁、link 和 OMP 指引。`ensure` 幂等且不会显示隐藏的 pet。可选 attach 自动启动需真正支持钩子的宿主。lifecycle 文件原先不存在时 Herdr 退出策略默认为 true；已有文件缺少该键时为 false。写入保留显式策略与未知字段。Tern 是独立 consumer，不自动启动共享应用。

## 源码 / link

先在 **desktop-pet Core** checkout 执行 `git checkout --detach 2936b2c600235ec8d15d1248ef3380e0a8dca8c0 && bash scripts/install.sh --source` 安装或构建 app，再 link **此集成 checkout**。`plugin link` 不构建。当前本地 consumer metadata 固定实际公开 Core4.8；上述 Core4.7同实例复用仍是历史证据，不是新版本运行验收。无新集成标签/此 consumer 的 native build。历史集成4.1/Core4.2并发 bootstrap/wrapper 与 Core4.5 Hide→ensure 证据保持原范围。参阅[独立 checkout 流程](integrations/herdr/README.md#build-in-separate-checkouts)；plugin 安装不 patch Herdr。

通用[使用指南](https://github.com/hanbong5938/desktop-pet/blob/main/docs/usage.zh-Hans.md)、[CLI](https://github.com/hanbong5938/desktop-pet/blob/main/docs/cli.md)、[开发](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.md)及原生功能、provider 详情均属于独立 core。

## 所有权

本仓库拥有 adapter、manifest、轻量 wrapper、Herdr/OMP 集成、集成验证及冻结历史。应用、UI、rig、美术、包装、源码构建属于 [desktop-pet](https://github.com/hanbong5938/desktop-pet)。`.herdrchar`、pack v5、`default@0`、rig ABI 及 [herdr-characters](https://github.com/hanbong5938/herdr-characters) 权威不变。公开正文、标签、素材保持不变。修正 v0.4.1 已通过整个 filesystem/index/archived-tree 所有权检查，不包含原生 bundle/archive/rig 产品。

## 美术与许可

[软件许可](LICENSE.txt)与美术／模型条款**彼此独立**：MIT 软件许可并不重新授权角色美术。内置露贝莉娅的[许可](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/LICENSE.txt)、[署名](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/ATTRIBUTION.txt)、[来源记录](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/source-record.json)及[背景修复记录](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/background-repair.json)具有权威性。`LICENSE.txt` 还保留原 Coding Cat 声明；其美术／源码已移至[角色仓库](https://github.com/hanbong5938/herdr-characters)。请保留每个可选角色包自己的条款。美术所有者的批准不授予模型权利；Qwen 的研究许可仅为制作记录，应用未附带模型引擎／权重。参阅[角色制作与许可（英语）](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.md#character-authoring)。
