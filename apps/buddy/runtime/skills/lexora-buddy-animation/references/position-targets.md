# Position Targets

## Built-In Targets

- `center`：当前 monitor workarea 的中心，桌宠完整可见。
- `home`：当前 monitor workarea 的右下休息位，桌宠完整可见。
- `edge left/right/top/bottom`：当前 monitor workarea 的可见边缘。
- `position x/y`：用户明确给出的坐标；runtime 仍会按可见策略夹紧。
- `x`：只改变横向位置，纵向沿用当前桌宠位置；主要用于兼容窗口侧边命令。
- `windowAnchor`：窗口语义目标，只在 `sequence` 里使用；当前限定为活动窗口后方露出头部，不是普通屏幕边缘移动。

`windowAnchor` 参数：

- `selector.kind` 固定为 `activeWindow`。
- `edge` 可以是 `auto`、`left`、`right`、`top` 或 `bottom`；`auto` 让 runtime 根据窗口位置和遮挡空间决定露头边。
- `reveal` 固定为 `head`。
- `durationMs` 范围是 `500..15000`，表示到达后保持窗口层级/位置的时间。

如果当前桌面环境不能提供安全的活动窗口几何，`windowAnchor` 应失败为 target unavailable；不要在外部脚本里自行猜测窗口位置。

## Original

`original` 不是 runtime 的固定位置。它表示“本次动作开始前的位置”，必须由脚本先调用 `state` 保存 snapshot。

需要“回到原地”时，用高层命令：

```bash
node <skill_dir>/scripts/lexora-buddy-pet.mjs perform center-cast-return-sleep --animation cast --duration-ms 2000
```

或用 `sequence` 显式 snapshot：

```json
{
  "steps": [
    { "type": "snapshot", "name": "original" },
    { "type": "move", "target": "center" },
    { "type": "animation", "animation": "celebrate", "durationMs": 2000 },
    { "type": "move", "target": { "kind": "snapshot", "name": "original" }, "after": "sleep" }
  ]
}
```

不要把 `original` 解释为默认右下角，也不要在没有 snapshot 的情况下声称已经回到原位。
