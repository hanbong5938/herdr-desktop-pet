# Desktop Pet — Herdr integration

[English](readme.md) · [한국어](readme.ko.md) · [日本語](readme.ja.md) · [简体中文](readme.zh-Hans.md)

**发布边界：**旧版 [Herdr v0.3.6](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/v0.3.6) 已发布，采用 protocol 3 / observation capability 1。独立 Desktop Pet v0.4.0（protocol 4 / capability 2 / integration API 1）与迁移 bridge v0.3.7 是经过审阅的候选版本，公开安装与运行验证尚未完成。以下是中立产品切换指引；文档、标签、源码版本号都不证明已发布。

仅安装需要的集成。Herdr 是轻量消费者；Tern 不下载、构建、安装或自动启动原生应用。插件更新与卸载只影响集成，不删除共享应用、配置档或角色。常规应用更新不重新安装 Herdr 插件。

[Desktop Pet / core app](https://github.com/hanbong5938/desktop-pet/blob/main/readme.zh-Hans.md) · [Herdr host / patch / OMP / link](integrations/herdr/README.md) · [Legacy A → B migration](https://github.com/hanbong5938/desktop-pet/blob/main/docs/migrations/desktop-pet.md)

[Core 应用网站](https://hanbong5938.github.io/desktop-pet/zh-Hans/) · [Core Wiki](https://github.com/hanbong5938/desktop-pet/wiki) · [Herdr 集成网站](https://hanbong5938.github.io/herdr-desktop-pet/zh-Hans/)。旧网站仅提供集成与迁移入口，不复制应用 UI 或美术。

中立发布及配方实际公开之后，公开安装命令为：

```sh
brew install hanbong5938/tap/desktop-pet
herdr plugin install hanbong5938/herdr-desktop-pet
herdr plugin action invoke start --plugin desktop-pet
```

Apple Silicon Mac、macOS 13 或更新版本。源码构建需 Rust/Cargo、Bun、Node.js/npm、Xcode Command Line Tools，预构建无需这些工具。应用为 ad-hoc 签名，未经公证。Herdr 快照需兼容本地服务器（0.9.0+），回复另需 `agent.prompt`。可选 client-attach 自动启动需官方 Herdr 0.9.3 应用[集成仓库的补丁和宿主构建指引](https://github.com/hanbong5938/herdr-desktop-pet/blob/main/integrations/herdr/README.md)，或实际支持钩子的宿主。原版 0.9.0/0.9.3 没有该钩子，插件安装不修改宿主。

预构建与源码安装在 **core** 检出目录中执行，而不是 Herdr 集成目录。默认仅预构建，下载或验证失败不会暗中回退源码编译。`--source` 明确仅构建当前 core，并使用该目录的 `dist/DesktopPet.app`。

## Herdr consumer 约定

本仓库不构建或复制 `DesktopPet.app`、原生软件包或角色素材。仅集成 manifest、wrapper 与 adapter 已迁入此检出目录；原生源码、美术与工具由 core 维护。生成 consumer pin 与公开安装仍需真实公开的 core installer/checksum 及审阅后的 Release API 元数据。目标链接与本地源码不能证明已发布。

准确发现兼容共享应用的顺序为：显式绝对可执行路径 → 已验证的真实账户主目录共享 locator → PATH 中原生 `desktop-pet` → 实际 Brew prefix → 真实账户主目录的 `Applications/DesktopPet.app`。无效显式路径或 locator 不会回退至其他安装。仅应用缺失时允许通过固定且 digest 已验证的 **core** installer bootstrap；不兼容安装需显式原生升级或修复，不在此构建产品或静默 fallback。

Herdr 配置档优先级为注入的 `HERDR_PLUGIN_CONFIG_DIR` / `HERDR_PLUGIN_STATE_DIR` → 显式 `--config-dir` / `--state-dir` → Herdr XDG/HOME 默认值。wrapper 将两个解析后路径显式传入 core，不移动或合并配置档。后端来源通过 `--herdr-socket ABS` 传入，不是旧 `--socket` 或 pet 的私有控制套接字。`status` 显示实际运行映像及配置档，而不只是 CLI。

[Herdr 指南](integrations/herdr/README.md) 保留现有 21 个 action ID 及其角色显示、气泡、生命周期语义，以及宿主构建、补丁、link 和 OMP 指引。`ensure` 幂等且不会显示隐藏的 pet。可选 attach 自动启动需真正支持钩子的宿主。lifecycle 文件原先不存在时 Herdr 退出策略默认为 true；已有文件缺少该键时为 false。写入保留显式策略与未知字段。Tern 是独立 consumer，不自动启动共享应用。

## 源码 / link

先在 **desktop-pet core** 检出目录执行 `bash scripts/install.sh --source` 安装或构建应用。选择兼容应用后 link **此集成检出目录**，而非 core。`plugin link` 不执行构建。公开/bootstrap 使用仍需真实 consumer pin 生成。参阅[独立检出流程](integrations/herdr/README.md#build-in-separate-checkouts)；插件安装不修改 Herdr 宿主。

通用[使用指南](https://github.com/hanbong5938/desktop-pet/blob/main/docs/usage.zh-Hans.md)、[CLI](https://github.com/hanbong5938/desktop-pet/blob/main/docs/cli.md)、[开发](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.md)及原生功能、provider 详情均属于独立 core。

## 所有权

本仓库拥有 `native-adapter`、`herdr-plugin.toml`、轻量安装及运行 wrapper、`integrations/herdr`、`integrations/omp`、集成验证及冻结历史发布。应用、UI、rig、美术、软件包、源码构建属于 [desktop-pet](https://github.com/hanbong5938/desktop-pet)。`.herdrchar`、pack v5、`default@0`、rig ABI 及 [herdr-characters](https://github.com/hanbong5938/herdr-characters) catalog/gallery 权威保持不变。历史 [v0.3.6](https://github.com/hanbong5938/herdr-desktop-pet/releases/tag/v0.3.6) 及更早发布正文、标签、素材不可变；仅集成 v0.4.0 不包含原生 archive。

## 美术与许可

[软件许可](LICENSE.txt)与美术／模型条款**彼此独立**：MIT 软件许可并不重新授权角色美术。内置露贝莉娅的[许可](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/LICENSE.txt)、[署名](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/ATTRIBUTION.txt)、[来源记录](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/source-record.json)及[背景修复记录](https://github.com/hanbong5938/desktop-pet/blob/main/assets/rubelia-default/background-repair.json)具有权威性。`LICENSE.txt` 还保留原 Coding Cat 声明；其美术／源码已移至[角色仓库](https://github.com/hanbong5938/herdr-characters)。请保留每个可选角色包自己的条款。美术所有者的批准不授予模型权利；Qwen 的研究许可仅为制作记录，应用未附带模型引擎／权重。参阅[角色制作与许可（英语）](https://github.com/hanbong5938/desktop-pet/blob/main/docs/development.md#character-authoring)。
