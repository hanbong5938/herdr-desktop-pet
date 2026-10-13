# Herdr 集成使用指南

[English](usage.md) · [한국어](usage.ko.md) · [日本語](usage.ja.md) · [简体中文](usage.zh-Hans.md)

当前原生应用操作、角色、配置档、提供商连接与恢复由独立 Desktop Pet core 维护。

- [当前应用使用指南](https://github.com/hanbong5938/desktop-pet/blob/main/docs/usage.zh-Hans.md) · [应用网站](https://hanbong5938.github.io/desktop-pet/zh-Hans/)
- [Core CLI（英文）](https://github.com/hanbong5938/desktop-pet/blob/main/docs/cli.md#english) · [Core Wiki](https://github.com/hanbong5938/desktop-pet/wiki)
- [Herdr action、hook、生命周期、宿主 patch、link 与 OMP](../integrations/herdr/README.md)
- [旧安装 A → B 迁移](https://github.com/hanbong5938/desktop-pet/blob/main/docs/migrations/desktop-pet.md)
- [Tern Connect / Configure](https://github.com/hanbong5938/tern-desktop-pet)

通过 core installer 单独安装应用，或在 formula 实际发布后使用 `brew install hanbong5938/tap/desktop-pet`。可选集成为 `herdr plugin install hanbong5938/herdr-desktop-pet` 与 `tern plugin install https://github.com/hanbong5938/tern-desktop-pet`。Herdr 使用共享应用；Tern 不安装或自动启动应用。选择实际 config/state 双路径，不移动或合并配置档。移除插件保留应用、配置档与角色。

[发布边界](../readme.zh-Hans.md)：链接与命令不能证明实际 Release、formula、Pages 或 Wiki 已公开。这里不声称真实 Tern/OMP 验收成功。冻结的 [v0.3.6 文档](https://github.com/hanbong5938/herdr-desktop-pet/tree/v0.3.6/docs)与[历史发布](releases/README.md)仅保留历史，不是当前安装指引。
