# Lexora Desktop

`apps/buddy` 是 Lexora 的 Electron 桌面应用，包含对话界面、Buddy 桌宠和本地 Codex Runtime。

关闭窗口仅隐藏 Desktop；退出由托盘控制。双击桌宠可重新显示 Desktop。

当前版本提供本地对话、项目目录授权、附件、审批和对话历史。

本地配置保存在 `~/.lexora/config.toml`，会话、运行事件和附件保存在 `~/.lexora/buddy/`。

## 开发

```bash
pnpm dev:buddy
```

构建与发布见 [`packaging/buddy/README.md`](../../packaging/buddy/README.md)。
