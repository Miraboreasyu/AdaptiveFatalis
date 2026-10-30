# 黑龙离线 AI 模型：查询独立出招表、只改概率字节

本次是离线第一步，不是新的实战插件版本。没有安装进游戏，没有接入旧的全局动作反查学习器。旧运行时的归因问题尚未因此自动修好。

## 首选接口：直接调用 Rust API（后续代码不调用 EXE）

当前主接口为 crate 根导出的 `AiDocument`，实现在 `src/ai_document.rs`。生命周期是：THK bytes -> 解析后的节点/段/独立概率表 -> 在内存里修改概率 -> 由节点/段序列化成 THK bytes。查询读取同一个已修改模型，不再从外部 JSON 修改请求驱动主流程。

```rust
use fatalis_adaptive::{AiDocument, geometry::Query};

fn update(main: &[u8], global: &[u8]) -> Result<Vec<u8>, String> {
    let mut ai = AiDocument::parse(main, global)?;
    let candidates = ai.query(&Query {
        distance: 700.0, angle_cw: 10.0, entry_node: 11,
    })?;
    // 后续学习器在内存中按各张表独立计算新分布；此处只是手动例子。
    let _ = candidates;
    ai.set_weights("main:n105:s1", &[45,20,0,0,20,10,5,0])?;
    let current_table = ai.table("main:n105:s1").unwrap();
    assert_eq!(current_table.branches[0].weight, 45);
    ai.to_thk()
}
```

`tables()` / `table(id)` 为只读语义视图；使用 `set_weights` 原子地验证/更新整张分布，同步更新对应 Segment 的参数。内部 Project 不暴露可变引用，避免表与段出现两份互相矛盾的概率。失败的修改不改变模型。

`to_thk()` 由 `Thk::to_bytes()` 根据当前解析字段写出，不调用旧 patch 函数，不接收编辑请求或文件偏移。未知字段和 padding 作为无损保留数据写回；已解析字段按模型重新编码。THK 本身就是编译后的二进制，因此这个输出过程就是这里的编译/序列化，无须另起文本编译器。

需要继续编辑上次保存的文件时，使用 `AiDocument::parse_current(original_main, global, current_main)`，导入后的概率就是模型当前状态。原版仅用作身份/结构及原始零选项校验。当前版本保持原布局，不支持增删节点/段，也不是通用 NACK 编译器。

`weight_edit` 已降为兼容适配层，内部调用 AiDocument，不是后续学习器的主接口。离线 CLI 只保留为可选人工诊断工具，通过 Cargo feature `offline-cli` 显式开启才构建；本轮没有再构建/交付 EXE。旧实战插件尚未接入新对象。

## 你要的对象

学习/编辑的身份是 `main:n105:s1` 这样的**一张 random/elser 表**，表内每个选项保留直接调用的 Global 节点。`n105` 是主 THK 原始节点索引，`s1` 是该节点的原始段索引；不是 Global 的压缩中文编号。相同 Global 在另一表出现，不共用记录；本版没有任何跨表学习分摊。

## 代码在做什么

- `src/thk.rs`：从原始二进制读取节点/段，枚举全部 603 张主 THK random 表；原版主文件和 Global 均校验固定 SHA256。每个权重为选项头段的 `+12..+16` 四字节。
- `src/geometry.rs`：按 main 的真实 local 调用向下遍历，继承调用者的路径条件。if/elif/else 逐级消耗未命中的状态；else 带上此前**所有**条件的否定。距离函数 8/9、顺时针角度函数 22 以查询值求真值；其他状态条件保持符号形式，不擅自当作 true 或 false。完全相反的同一符号条件排除。每张表返回 options 与 routes；同一表的多条路径合并展示，表本身不合并。
- `TableOption.global_node`：直接引用的真实 Global 节点索引，先按 external_id 查 Global 节点表得到，不把 external_id 错当索引。零概率选项、空选项照原顺序保留。复杂选项头若没有直接 Global 调用，字段为 null，不编造终端招式。
- `ordinary_table_ids`：主 node11 的普通距离分发目标节点 75..125 中命中的表；`tables` 同时保留前置特殊/控制表。这个分组是基于当前原版黑龙结构的显式标注，不是通用 THK 定律。
- `src/ai_document.rs`：统一可编辑对象、查询、概率修改与 THK 输出的首选 API。
- `src/weight_edit.rs`：兼容旧诊断调用的适配层；显式指定 table ID 和完整整数权重数组。校验输入当前文件 hash、长度、总和 100、范围 0..100；保持原始零权重选项为零。允许从已经改过权重的当前文件继续编辑，不复原其他表的修改。最终再次比较全文件，任何非本次指定权重字段的变化都是错误。
- `src/offline.rs`：独立 `fatalis-ai.exe` 命令，不读取游戏进程、不加载插件、不部署文件。query 查表，template 导出可编辑权重，edit 生成新文件和逐字节变更报告。输出存在则拒绝覆盖。

## 真实查询例子

从主 node11 查询：距离 700、顺时针角度 10°。

node105 对应路径包含：非二足分支 -> 四足分支；距离 <=500 为假、<=1000 为真；P1 为真；RV11>=1 为假；进入 node105；angle(330,30) 为真。所以命中 `main:n105:s1`，而不是 node105 的其他四张表。

本模型暂不提供阶段/姿态/RV11 的观测输入，保留这些未知条件。因而同一查询的普通列表有 9 张表：n76、82、87、92、98、105、110、116、122 各自 s1。它们是状态条件不同的候选表，不是同一表的 9 个选项。每张表各自有自己的权重和直接 Global 选项。前置特殊逻辑也查到表，所以 entry11 的完整 `tables` 有 54 张；不能把它误称为普通出招的 54 个平行选择，更不能拿它做全局均分。

距离改成 1200，node105 完全不在结果中；node106 等中距离表接替。指定 entry105 是单节点审查，不代表仅凭角度证明游戏已经进入 node105。

只查询 node105，距离 700：

| 角度 | 独立表 | 原始权重 |
|---|---|---|
| 10° | main:n105:s1 | 35/30/0/0/20/10/5/0 |
| 60° | main:n105:s11 | 25/20/0/20/20/10/5/0 |
| 300° | main:n105:s21 | 25/20/0/20/20/10/5/0 |
| 180° | main:n105:s31 | 10/10/0/10/20/30/20/0 |
| 120° | main:n105:s41 | 10/10/0/10/20/10/25/15/0 |

第一张表直接 Global 索引依次为 102、103、101、110、116、113、124、空。即使权重为零也保留，避免索引错位。

## 真实编辑例子

`examples/geometry/edit-105-front.json` 只指定 s1 改为 45/20/0/0/20/10/5/0。结果长度仍 960004，实际差异仅文件零基偏移 426268（35 ->45）和426396（30 ->20）。其余所有字节完全一致，尤其其他四张角度表不变。权重字段是四字节，本例只有低字节产生差异。

输出 hash：0287d748b247f0059060094275ae2ccd23e6c14af207d78423473644871980e0。

## 使用（从仓库根目录，PowerShell）

```powershell
$ai = '.\tools\fatalis_adaptive\target\release\fatalis-ai.exe'
$main = '.\research\fatalis_thk\em013_00.thk'
$global = '.\research\fatalis_thk\em013_55.thk'
& $ai query $main $global 700 10 11
& $ai query $main $global 700 10 105
& $ai edit $main $global $main '.\tools\fatalis_adaptive\examples\geometry\edit-105-front.json' '.\my-edited-main.thk'
```

`query MAIN GLOBAL DISTANCE ANGLE [ENTRY=0] [CURRENT_MAIN]` 的 CURRENT_MAIN 可查看已改权重，而路由结构仍按经校验的原版构造。距离使用游戏 AI 原始单位，角度沿用 self.angle_2d_cw_between 的顺时针坐标，不能直接喂屏幕方位。

`template MAIN GLOBAL [CURRENT_MAIN]` 输出 hash + 全部表当前权重，编辑前保留需要修改的表即可。

## 不能省略的边界

这是**固定距离/角度下的结构路径投影**，不是完整 THK VM。阶段、部位破坏、姿态、寄存器未知条件保留，而不模拟状态写入。Global 调用不递归展开动作，也不执行其中的 reset/移动/抢占；local 被调节点的返回后状态和抢占效果不传回调用者。因此 root0/entry11 的结果是带条件的结构候选，不是证明实机此时会运行到该表。条件头与 inline 几何门控已处理；循环截断会有 warning，超过预算会报错、不静默返回部分表。

角度恰在 30° 等端点时，包含性尚未实测：保留可能的相邻表，必要时也保留 else（两边都严格不含端点的可能性），并输出 warning。

本版尚未解决 ActionID/动作序列 -> 所选 Global 节点。没有用递归动作集合冒充这个识别结果。后续接学习时，应先几何查表，再在每张表中定位实际 Global；没有对应选项的表不更新，有对应选项的各表分别更新，不能继续复用旧的全局出口均分。

## 验证

`cargo test` 覆盖原项目和新离线回归；新增真实节点五角度表、上游距离排除、端点歧义、限定权重字节/多次编辑保留已有修改、过期 hash/非法总和/激活零选项拒绝，以及条件头同行调用和 inline 条件的控制语义。

`py tools/fatalis_adaptive/check_geometry_examples.py` 调用 release CLI 生成真实查询输出、编辑报告并逐字节检查。全部测试离线，不证明游戏下次任务的资源重载行为。
