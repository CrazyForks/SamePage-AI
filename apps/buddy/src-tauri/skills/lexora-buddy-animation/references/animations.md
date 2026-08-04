# Animations

## Semantic Mapping

- 开心 / 庆祝：`celebrate`
- 施法 / 蓄力 / 酷炫动作：`cast`
- 睡觉：`sleep`
- 醒来：`wake`
- 思考：`thinking`
- 工作 / 专注：`working`
- 解释：`explain`
- 安慰 / 放松：`reassure`
- 好奇 / 看看：`curious`
- 难过：`sad`
- 同意 / 认可：`approval`
- 跑动：runtime 自动使用 `run_left` / `run_right`

## Runtime Inventory

不要在 skill 中维护完整 runtime animation name 清单。安装版本、manifest 和动作资产会持续演进，调用前通过 runtime 查询当前能力：

```bash
node <skill_dir>/scripts/lexora-buddy-pet.mjs capabilities
```

如果用户要求的动作没有稳定映射，不要发明动画名。只有语义确实等价时才选择已有动作；否则说明当前 runtime 没有该动作。尤其不要在正式 dance 动作缺失时把 `celebrate` 声称为跳舞。
