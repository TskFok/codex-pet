# 月影 Codex 宠物设计

## 目标

根据 `/Users/ushopal/Downloads/1.jpg` 和 `/Users/ushopal/Downloads/2.jpg` 生成一个可安装的 Codex 宠物。宠物命名为“月影”，输出完整九状态动画图集、`pet.json`、QA 联系表和预览动图。

## 视觉方向

采用贴近参考图的 Q 版动漫人物宠物风格，保留这些主要识别点：

- 酒红色长发，侧边高束发和分段长辫。
- 闭眼得意或从容的小表情，粉色脸颊。
- 黑色短上衣、黑金披风、暖橙色披风内衬。
- 金色珠饰、黑色短靴和星形小装饰。
- 小比例、全身可读、适合 `192x208` 单元格的紧凑轮廓。

生成结果不得保留参考图里的水印、网格背景、字幕、黑边或任何可读文字。

## 输出结构

新资源放在 `pet-output/yueying-cape-dancer/`：

- `run/` 保存生成运行产物、QA 输出和最终图集。
- `package/` 保存可安装的 `pet.json` 与 `spritesheet.webp`。

不会覆盖已有 `pet-output/zhizhi-branch-guardian/`、`pet-output/xiao-jiao-long-run/` 或 `pet-output/qian-run/`。

## 状态覆盖

生成 Codex 当前宠物协议的九行状态：

- `idle`
- `running-right`
- `running-left`
- `waving`
- `jumping`
- `failed`
- `waiting`
- `running`
- `review`

每个状态都必须保持同一角色身份、同一服饰和同一画风。`running-left` 只有在镜像不会破坏身份时才从 `running-right` 派生，否则单独生成。

## 验收

完成后必须生成并检查：

- `final/spritesheet.webp`
- `final/validation.json`
- `qa/contact-sheet.png`
- `qa/previews/*.gif`
- `package/pet.json`
- `package/spritesheet.webp`

验收标准是图集尺寸和透明单元格通过脚本校验，联系表中九个状态不漂移、不裁切、不带背景文字，并且预览动图能体现各自状态。
