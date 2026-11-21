# Agent Notes 从"格式约定"改为"完成闸门"

- **Date**: 2026-11-19
- **Status**: done
- **相关**: `AGENTS.md`（Agent Notes 约定）、`docs/todo.md`

## 背景

此前多轮功能改动（插件、预览、剪贴板等）都没有在 `.agents/notes/` 留下任何 note。
复查 `AGENTS.md` 后发现不是执行者单方面遗漏：原文只定义了 lifecycle/class 编码、
存放路径和短 note 骨架，**没有一句说明"什么时候必须写"以及"不写会怎样"**。
唯一带祈使语气的句子是"目录随第一条 note 一并创建，不预建空目录"，
实际效果更像是"先别建目录"，反而削弱了记录义务。

## 改动

在 `AGENTS.md` 的 Agent Notes 约定中补充：

- "何时必须写"：feature / bug-fix / simplification / architecture / process / testing
  在汇报完成前必须落 note，没有 note 视为未完成；
- 明确豁免：纯文案、纯格式、依赖升级、一次性排障脚本；
- 明确 `proposed → implemented` 的迁移方式，要求 `Status` 与目录一致；
- 澄清"不预建空目录"不等于"可以不写"。

## 判据

判断是否需要 note 的标准不是"改动大小"，而是"是否改变了事实、判据，或别人后续要遵守的约束"。
