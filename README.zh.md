<p align="center">
  <img src="assets/logo.gif" alt="FLUX Validator Logo" width="400">
</p>

<p align="center">
  <a href="README.md">DE</a> |
  <a href="README.en.md">EN</a> |
  <a href="README.fr.md">FR</a> |
  <a href="README.es.md">ES</a> |
  <a href="README.ja.md">JA</a> |
  <strong>ZH</strong>
</p>

# FLUX — AI 原生计算基底

**FLUX** 是一种执行架构，AI系统（LLM）以 FTL（FLUX Text Language）生成计算图，经过形式化验证后编译为最优机器码。

**LLM 生成 FTL 文本。系统编译为二进制。形式化验证。最优。**

## 设计公理

```
1. 编译时间无关紧要               → 穷举验证、超级优化
2. 人类可读性无关紧要             → LLM 使用 FTL（结构化文本）工作，
                                    系统编译为二进制
3. 人类补偿措施不需要             → 无调试、无异常处理、
                                    无防御性编程
4. 代码生成性能是次要的           → 无限 LLM 迭代次数，
                                    无限分析深度
5. 鼓励创造力                     → AI 应当发明新颖的解决方案，
                                    而非仅仅复制已知模式
6. 验证中的实用主义               → 分级证明策略，带超时，
                                    不可判定 → 升级处理，而非无限循环
```

## 架构

```
需求（自然语言，不在范围内）
    │
LLM（程序员 — 替代人类）
    │  FTL（FLUX Text Language） — 结构化文本
    ▼
FLUX 系统
    ├─ FTL 编译器（文本 → 二进制 + BLAKE3 哈希）
    ├─ 验证器（结构 + 类型 + 效果 + 区域）
    │    失败 → 向 LLM 返回 JSON 反馈（附建议）
    ├─ 合约证明器（分级：Z3 60秒 → BMC 5分钟 → Lean）
    │    反证 → 向 LLM 提供反例
    │    不可判定 → 向 LLM 提供提示或孵化
    ├─ 池 / 进化（用于发明/发现）
    │    向 LLM 提供适应度反馈（相对指标）
    ├─ 超级优化器（3级：LLVM + MLIR + STOKE）
    │    热路径最优，其余 LLVM -O3 质量
    └─ MLIR → LLVM → 本地机器码
    │
┌───┴────┬──────────┬──────────┐
ARM64   x86-64    RISC-V     WASM
```

## 节点类型

| 节点 | 功能 |
|------|------|
| **C-Node** | 纯计算（ADD、MUL、CONST、...） |
| **E-Node** | 副作用，恰好2个输出（成功 + 失败） |
| **K-Node** | 控制流：Seq、Par、Branch、Loop |
| **V-Node** | 合约（SMT-LIB2）— 编译前必须证明 |
| **T-Node** | 类型：Integer、Float、Struct、Array、Variant、Fn、Opaque |
| **M-Node** | 内存操作（绑定到区域） |
| **R-Node** | 内存生命周期（竞技场） |


## 核心原则

**LLM 作为程序员：** LLM 替代人类程序员。它提供 FTL 文本（非二进制，非哈希）。系统将 FTL 编译为二进制图，计算 BLAKE3 哈希，并返回 JSON 反馈。

**完全正确性：** 每个编译后的二进制文件都经过形式化验证。零运行时检查。合约通过分级证明策略（Z3 → BMC → Lean）得到证明。

**探索性合成：** AI 不是生成一个图，而是数百个。正确性是过滤器，创造力是生成器。遗传算法（GA）是主要的创新引擎；LLM 提供初始化和定向修复。

**超级优化：** 3级（LLVM -O3 → MLIR 级别 → STOKE）。热路径优于手写汇编。现实预期：相比纯 LLVM -O3 整体提升 5-20%。

**内容寻址：** 无变量名。身份 = 内容的 BLAKE3 哈希（由系统计算）。相同计算 = 相同哈希 = 自动去重。

**生物变异模型：** 有缺陷的图被隔离在孵化区进行进一步开发。变异之上的变异可以将"坏的"变成"特别的"。只有最终二进制文件必须可证明正确 — 通往目标的路径可以经过错误。

## 文档

- **[FLUX v3 规范](docs/FLUX-v3-SPEC.md)** — 当前规范（18个章节）
- **[FLUX v2 规范](docs/FLUX-v2-SPEC.md)** — 先前版本（有人类让步）
- **[专家分析](docs/ANALYSIS.md)** — 3个专业代理的评估（第2轮）
- **[Hello World 模拟](docs/SIMULATION-hello-world.md)** — 从需求到机器码的管道
- **[Snake Game 模拟](docs/SIMULATION-snake-game.md)** — 带声音的复杂示例

## 示例

- [`examples/hello-world.flux.json`](examples/hello-world.flux.json) — Hello World（v2 JSON 格式）
- [`examples/snake-game.flux.json`](examples/snake-game.flux.json) — Snake Game（v2 JSON 格式）

*注意：v3 使用 FTL（FLUX Text Language）而非 JSON。示例展示的是 v2 格式。*

## 需求类型

```
翻译       "用归并排序排序"                → 直接合成（1个图）
优化       "尽可能快地排序"                → 帕累托选择（多个变体）
发明       "改进 sort()，发明新方法"       → 探索性合成 + 进化
发现       "找到具有属性 X 的计算"         → 图空间中的开放搜索
```


## 许可证

MIT

## 致谢
- Bea — Logo
- Gerd — 灵感
- Michi — 评论
