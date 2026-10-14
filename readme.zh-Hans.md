# Desktop Pet — Herdr integration

[English](readme.md) · [한국어](readme.ko.md) · [日本語](readme.ja.md) · [简体中文](readme.zh-Hans.md)

**发布边界：**[Core v0.4.2](https://github.com/hanbong5938/desktop-pet/releases/tag/v0.4.2) 已真实公开：源码 `cb10ce2c3cc65402cbb914d60301f7627d5546d5`，Release `412241744`，protocol 4 / capability 2 / integration API 1。当前元数据及证据已匿名获取并审阅，属于真实 v0.4.2；v0.4.1 standalone、Manager 证据仅是历史记录。Herdr v0.4.0 的生成原生 `dist` 源码缺陷保留不变的公开标签、正文和素材。[修正 Herdr v0.4.1 候选](docs/releases/v0.4.1.md) 已通过 source、TypeScript、adapter 105 项验证，但尚无标签或 Release，新的 Herdr ID／最终 SHA 未知。Bridge v0.3.7 也未公开，必须严格固定真实 Core 及将来真实发布的修正 Herdr ref。Tern v0.2.0 保持历史兼容 Core v0.4.1 pin。真实自有 Brew v0.4.1 → v0.4.2 已达到相同/default 配置档 Auto UI/control readiness（PID `6863`，宿主 0），但 standalone native-update、迁移／双宿主完成尚未证明。

仅安装需要的集成。Herdr 是轻量消费者；Tern 不下载、构建、安装或自动启动原生应用。插件更新与卸载只影响集成，不删除共享应用、配置档或角色。常规应用更新不重新安装 Herdr 插件。

[Desktop Pet / core app](https://github.com/hanbong5938/desktop-pet/blob/main/readme.zh-Hans.md) · [Herdr host / patch / OMP / link](integrations/herdr/README.md) · [Legacy A → B migration](https://github.com/hanbong5938/desktop-pet/blob/main/docs/migrations/desktop-pet.md)

[Core 应用网站](https://hanbong5938.github.io/desktop-pet/zh-Hans/) · [Core Wiki](https://github.com/hanbong5938/desktop-pet/wiki) · [Herdr 集成网站](https://hanbong5938.github.io/herdr-desktop-pet/zh-Hans/)。旧网站仅提供集成与迁移入口，不复制应用 UI 或美术。

Core Pages 已真实部署 `cb10ce2`；此 Herdr thin-site 候选尚未部署，公开源码仍为历史 `f65f123`。Core Release 作业成功，但后续 Wiki 作业因缺少专用 `WIKI_PUBLISH_TOKEN` 失败。完整 catalog 渲染不等于 Wiki 发布，链接也不证明 Core Wiki 已初始化或发布。

中立配方已指向真实公开的 Core v0.4.2（[tap commit](https://github.com/hanbong5938/homebrew-tap/commit/32919c745864b54a2e716fcc5a3714d0d38d3e84)）。父级已真实观察 Brew 升级、签名、receipt、derivation、`brew test` 与公开 wrapper readiness，范围限定为宿主 0。可选 Herdr 命令安装仓库的公开集成，**不是尚未公开的修正 v0.4.1 候选**：

```sh
brew install hanbong5938/tap/desktop-pet
herdr plugin install hanbong5938/herdr-desktop-pet
herdr plugin action invoke start --plugin desktop-pet
```

Apple Silicon Mac、macOS 13 或更新版本。源码构建需 Rust/Cargo、Bun、Node.js/npm、Xcode Command Line Tools，预构建无需这些工具。应用为 ad-hoc 签名，未经公证。Herdr 快照需兼容本地服务器（0.9.0+），回复另需 `agent.prompt`。可选 client-attach 自动启动需官方 Herdr 0.9.3 应用[集成仓库的补丁和宿主构建指引](https://github.com/hanbong5938/herdr-desktop-pet/blob/main/integrations/herdr/README.md)，或实际支持钩子的宿主。原版 0.9.0/0.9.3 没有该钩子，插件安装不修改宿主。

预构建与源码安装在 **core** 检出目录中执行，而不是 Herdr 集成目录。默认仅预构建，下载或验证失败不会暗中回退源码编译。`--source` 明确仅构建当前 core，并使用该目录的 `dist/DesktopPet.app`。

## Herdr consumer 约定

本仓库不构建或复制 `DesktopPet.app`、原生软件包或角色素材。集成 manifest、wrapper、adapter 属于此检出目录，原生源码、美术与工具属于 core。当前 `core-release.json` / `release-evidence/` 已包含审阅后的真实 Core v0.4.2 权威。固定 installer SHA-256 是 `a0ec644c3baecc7b81baddd29fbbbd3a8f25777e804a6d9940b78820bacc08ec`。[准确 installer/checksum/API 证据与限制](docs/releases/v0.4.1.md#actual-core-v042-authority-and-limits)区分真实发布与运行验收。默认 consumer 脚本要求真实 v0.4.2 权威，链接与本地源码不能替代证据。

准确发现兼容共享应用的顺序为：显式绝对可执行路径 → 已验证的真实账户主目录共享 locator → PATH 中原生 `desktop-pet` → 实际 Brew prefix → 真实账户主目录的 `Applications/DesktopPet.app`。无效显式路径或 locator 不会回退至其他安装。仅应用缺失时允许通过固定且 digest 已验证的 **core** installer bootstrap；不兼容安装需显式原生升级或修复，不在此构建产品或静默 fallback。

Herdr 配置档优先级为注入的 `HERDR_PLUGIN_CONFIG_DIR` / `HERDR_PLUGIN_STATE_DIR` → 显式 `--config-dir` / `--state-dir` → Herdr XDG/HOME 默认值。wrapper 将两个解析后路径显式传入 core，不移动或合并配置档。后端来源通过 `--herdr-socket ABS` 传入，不是旧 `--socket` 或 pet 的私有控制套接字。`status` 显示实际运行映像及配置档，而不只是 CLI。

[Herdr 指南](integrations/herdr/README.md) 保留现有 21 个 action ID 及其角色显示、气泡、生命周期语义，以及宿主构建、补丁、link 和 OMP 指引。`ensure` 幂等且不会显示隐藏的 pet。可选 attach 自动启动需真正支持钩子的宿主。lifecycle 文件原先不存在时 Herdr 退出策略默认为 true；已有文件缺少该键时为 false。写入保留显式策略与未知字段。Tern 是独立 consumer，不自动启动共享应用。

## 源码 / link

先在 **desktop-pet core** 检出目录执行 `bash scripts/install.sh --source` 安装或构建应用。选择兼容应用后 link **此集成检出目录**，而非 core。`plugin link` 不执行构建。此候选已完成真实 v0.4.2 consumer pin 生成，但修正集成的公开/bootstrap 验收仍需父级门禁与发布。参阅[独立检出流程](integrations/herdr/README.md#build-in-separate-checkouts)；插件安装不修改 Herdr 宿主。

通用[使用指南](https://github.com/hanbong5938/desktop-pet/blob/main/docs/usage.zh-Hans.md)、[CLI](https://github.com/hanbong5938/desktop-pet/blob/main/docs/cli.md)、[开发](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.md)及原生功能、provider 详情均属于独立 core。

## 所有权

本仓库拥有 `native-adapter`、`herdr-plugin.toml`、轻量安装及运行 wrapper、`integrations/herdr`、`integrations/omp`、集成验证及冻结历史发布。应用、UI、rig、美术、软件包、源码构建属于 [desktop-pet](https://github.com/hanbong5938/desktop-pet)。`.herdrchar`、pack v5、`default@0`、rig ABI 及 [herdr-characters](https://github.com/hanbong5938/herdr-characters) catalog/gallery 权威保持不变。所有公开正文、标签、素材均不可变。修正 v0.4.1 必须通过整个工作树、Git index 和归档 Git tree 的所有权检查，不包含原生 bundle、archive 或 rig 产品。

## 美术与许可

[软件许可](LICENSE.txt)与美术／模型条款**彼此独立**：MIT 软件许可并不重新授权角色美术。内置露贝莉娅的[许可](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/LICENSE.txt)、[署名](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/ATTRIBUTION.txt)、[来源记录](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/source-record.json)及[背景修复记录](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/background-repair.json)具有权威性。`LICENSE.txt` 还保留原 Coding Cat 声明；其美术／源码已移至[角色仓库](https://github.com/hanbong5938/herdr-characters)。请保留每个可选角色包自己的条款。美术所有者的批准不授予模型权利；Qwen 的研究许可仅为制作记录，应用未附带模型引擎／权重。参阅[角色制作与许可（英语）](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.md#character-authoring)。
