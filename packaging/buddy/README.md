# Lexora Desktop Packaging

本目录维护 Lexora Desktop 的构建、校验和发布脚本。产品入口见 [`apps/buddy/README.md`](../../apps/buddy/README.md)。

## 构建

```bash
pnpm --filter @lexora/buddy package:full
pnpm --filter @lexora/buddy package:pet
```

产物写入 `apps/buddy/dist-packages/`。`package:full` 包含 Electron Desktop、Rust Runtime 和桌宠；`package:pet` 仅包含独立桌宠。

## 校验

```bash
pnpm buddy:version:check
pnpm check:buddy
```

产品版本以 `apps/buddy/buddy.version.json` 为准。

## GitHub Release 与 AUR

`lexora-buddy-bin` 默认从 `haohaoxue-site/Lexora` 的 `v<version>` Release 下载完整 Desktop deb。运行 `Lexora Desktop Linux Build` workflow 时：

- 始终校验构建 deb 与 `PKGBUILD` 中的文件名、版本和 sha256；
- `upload_release_asset=true` 且 workflow 从 `master` 运行时，进入 `buddy-release` Environment，创建对应 Release（如不存在）并上传 deb；
- 发布完成后从 AUR 默认 URL 重新下载并复核 sha256，远端资产缺失或内容不一致都会令发布任务失败。

Actions artifact 仅用于 CI 留档，不能替代公开 GitHub Release 资产。
仓库侧必须为 `buddy-release` Environment 配置 required reviewers，并将 deployment branch 限定为 `master`。
