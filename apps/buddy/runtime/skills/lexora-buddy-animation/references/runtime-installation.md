# Runtime Installation

## Installed Environment

正式环境以 `packaging/buddy` 为准：

- deb/apt 包名：`lexora-buddy`
- Arch/AUR 包名：`lexora-buddy-bin`
- 主二进制：`lexora-buddy`
- 轻量桌宠控制入口预留名：`lexora-buddy-pet`

当前脚本连接 Buddy runtime 暴露的 native pet control socket。它是临时 IPC 端点，不是安装路径，也不是持久数据目录。

Linux 主路径：

```txt
$XDG_RUNTIME_DIR/lexora-buddy/native-pet.sock
```

没有 `XDG_RUNTIME_DIR` 时才降级到 uid 隔离的临时目录：

```txt
/tmp/lexora-buddy-uid-<uid>/native-pet.sock
```

也支持通过环境变量覆盖：

```bash
LEXORA_BUDDY_PET_SOCKET=/path/to/native-pet.sock node <skill_dir>/scripts/lexora-buddy-pet.mjs state
```

## Diagnostics

首次使用或失败时运行：

```bash
node <skill_dir>/scripts/lexora-buddy-pet.mjs diagnose
```

`diagnose.socket.connectable` 只表示 Unix socket 能建立连接；`diagnose.socket.responsive` 表示 runtime 已成功响应轻量 `state` 控制请求。`connectable=true` 但 `responsive=false` 时，按 stale / unhealthy sidecar 处理，不要继续把该 runtime 当成可控。

`diagnose.installation.packages.pacman.version` 和 `diagnose.installation.binaries[].sha256` 用于确认当前系统实际安装并运行的包身份。排查“已构建但未安装”或旧进程残留时，优先看这些字段，不要只看 socket 是否 responsive。

`diagnose.activeWindow` / `active-window` 在 KDE / Plasma 下会使用临时 KWin script + user journal token 读取活动窗口几何，成功时 `source` 为 `kwin-journal`。如果返回 `ok=false`，只能说明当前环境没有可安全读取的活动窗口几何；不要自行从标题、鼠标位置或屏幕边缘猜测窗口。

`launch` 同样以控制协议响应作为 ready 标准：只有轻量 `state` 请求成功时才返回 ready / reused。若主进程或 sidecar 已启动但控制协议不响应，`launch` 会返回 `ok=false` 并带 `responseError`，不要把单纯 socket connectable 当成安装验证通过。

如果 socket 不存在但已安装 Buddy，可以尝试：

```bash
node <skill_dir>/scripts/lexora-buddy-pet.mjs launch
```

如果没有安装二进制，提示用户通过 apt/deb 或 pacman/AUR 安装 Lexora Buddy。不要假设源码目录存在，也不要要求用户从仓库路径启动。
