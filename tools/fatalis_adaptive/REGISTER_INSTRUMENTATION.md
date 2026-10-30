# 全量随机分支插桩与精确学习：实现、接入及验证边界

本文件记录 `register` 模式。它把黑龙 THK 里**所有可直接包装的随机表正概率选项**都改成带“表号 + 出口号 + 计数器”的包装节点，运行时从 cThinkEm 派生六个物理寄存器，按事件结算双方掉血，战后逐表更新权重。目标是让玩家正常打，不需要刻意“引某招”。

当前离线覆盖：**591 张随机表、3282 个正概率选项**，生成约 **10.8 MB** 的插桩 THK。黑龙共有 603 张 random 表、3328 个正概率选项；未插桩的是 46 个带本地调用/附加元数据的选项，以及 12 张只含这类选项的外层选择表。它们不是普通直接出招入口，当前不猜测、不强行改写。

> 状态：离线构建、结构测试和学习测试通过；派生链与包装执行在 2026-10-30 的一次短局中以**旧 DLL + 30 选项包**实测到 2 个事件、`failed_reads=0`、`failed_context=0`。**全量 591 表/3282 选项的 10.8 MB THK 尚未在游戏中加载**，不能把旧包的局部成功当成全量验证。

## 一、数据流

1. `AiDocument::parse(main, global)` 建立可编辑模型。
2. `doc.to_instrumented_thk(&targets, Registers {...})` 输出插桩 THK + Manifest。
   - `targets` 为空表示自动发现全部可插桩表（591 张）。
   - 也可显式给出表 ID 子集；表号按排序后的目标集合分配。
3. `Manifest::option_map()` 得到 `packed_id -> (table_id, exit_index)`。
   - `packed_id = table_code * 100 + exit_index`。
   - `table_code` 是排序目标集合中的 0 基表号，3 位十进制；`exit_index` 是该表内 0 基出口号，2 位十进制。
4. 运行时每次轮询读取五个“表号/出口号”数字槽和一个计数器，组成 `RegisterSnapshot`，送入 `tracker.push(Some(snapshot))`；单次寄存器读取失败会跳过该轮采样（不会把失败区间硬计给旧选项），确认 HP 不完整时送 `None` 丢弃活动窗口。
5. 战斗结束/离开战斗后 `tracker.finish()`，`drain_completed()` 后调用 `register_learning::apply_hunt(...)`；每个事件只学习其明确指向的选项，`confidence=1`，不分摊到全局候选。
6. 更新后的 `current` 再次 `to_instrumented_thk`，同一目标集合得到下一场的 THK。

## 二、配置

最小配置（默认全部表、派生地址）：

```json
{
  "enabled": true,
  "allow_unverified_offsets": true,
  "register": {
    "table_code_slots": [14, 15, 16],
    "option_slots": [17, 18],
    "counter_slot": 19,
    "target_tables": []
  }
}
```

- `table_code_slots`：3 个槽，依次写表号的百/十/个位。
- `option_slots`：2 个槽，依次写出口号的十/个位。
- `counter_slot`：1 个槽，每次事件最后 `++`。
- 六个槽必须互不相同且在 `0..=21`。
- `target_tables` 为空 = 自动发现全部 591 张可插桩表；显式列出则只插这些表（仍按排序分配表号）。
- 六个地址字段都省略：每次轮询从已经识别的怪物派生。
- 六个地址字段都提供：显式覆盖，顺序为 5 个数字槽地址 + 1 个计数器地址。

显式覆盖示例（地址必须换成实测并验证过的值）：

```json
"addresses": [
  { "kind": "absolute", "address": 0 },
  { "kind": "absolute", "address": 0 },
  { "kind": "absolute", "address": 0 },
  { "kind": "absolute", "address": 0 },
  { "kind": "absolute", "address": 0 },
  { "kind": "module_chain", "offsets": [0, 0] }
]
```

- `absolute`：直接读该地址的 `i32`。
- `module_chain`：`base = GetModuleHandleW(NULL)`；对每个 offset 先加到当前指针、再解引用；最后一个 offset 解引用后的地址就是读取位置。
- 只给部分地址、数组长度不是 6、槽重复/越界都会拒绝启动。
- 含 `register` 段时强制 `allow_unverified_offsets=true`。

## 三、地址派生链（静态证据 + 一次旧包实机）

```
AIData   = *(monster + 0x12278)
cThinkEm = *(AIData + 0xAA8)
数字槽 i = cThinkEm + 0x640 + 4 * slot_i      // i32
计数器   = cThinkEm + 0x640 + 4 * counter_slot
校验      = *(cThinkEm + 0x630) == AIData
```

证据来自 `research/rebuild/MonsterHunterWorld.rebuilt.exe` 的调用点 `0x141bb683e..0x141bb6860`：分配 `0x7C0` 字节后调用 `0x14132b250` 初始化 cThinkEm，随后 `mov [rdi+0xAA8],rsi`（AIData→cThinkEm）和 `mov [rsi+0x630],rdi`（cThinkEm→AIData）。历史 THK logger 的源码把寄存器读作 `int32_t`：`cThinkEm + 0x640 + 4*index`（`.tmp/thk-logger-research/mhw-thk-logger/symbols.cpp:17-20`），并读取 `0..21`（同文件 `132-142`）。重建 exe 的 AI 解释器入口 `0x1413346b8` 把第一个参数保存到 `rbx`，随后反复读取 `[rbx+0x630]` 作为 AIData；这独立支持“`rbx` 是 cThinkEm、`+0x630` 是 AIData 反向指针”。

2026-10-30 的短局使用的是旧 DLL（`a9319f72...`，787968 B）与 30 选项包；日志出现 `failed_reads=0`、`failed_context=0`、`events=2`，说明该构建上派生链、插桩写入和读取至少跑通过。但旧包只有 node105 的五张表，不能证明 591 表的 10.8 MB 文件能正常加载。

派生失败（指针不可读、反向引用不匹配）时该次采样跳过并限频记录；不会把失败区间硬计给旧选项。首次成功解析会写 `register context resolved: monster=0x… code=(0x…,…) counter=0x… mode=… slots=[…]`，首次读到寄存器会写 `first register read: counter=… option_id=…`。前者证明链路可读，后者证明插桩确实写入了槽位。

## 四、为什么是六个物理槽，而不是申请更多寄存器

THK 头没有寄存器数量字段，`structCount` 是节点数；Leviathon 的 `declare` 只是把名字映射到固定物理槽，限制 `0..21`。没有证据表明改文件可以新增引擎槽位。

只读扫描 `research/fatalis_thk` 下 60 份 `em013_*.thk`（10060 个 segment）：脚本引用槽 `0..13`、`20`、`21`；`14..19` 未引用。启动时 `RegisterAudit` 会拒绝当前 main/Global 已引用的槽，但不会替你扫描其它 THK；这不是引擎安全保证。

## 五、原生指令与十进制编码

`:=` 是 ETL 扩展，未安装 ETL 时不会执行；默认实现只使用原版脚本已有的清零/自增和比较指令。每个包装节点执行：

```text
对 5 个数字槽中的每一个：
    清零（unary operator 1）
    ++，重复该位数字次（unary operator 0）
counter_slot ++（unary operator 0）
原来的 Global 调用
return
node-end
```

组合编码：

- 表号 3 位十进制：最多 999 张表；当前实际 591。
- 出口号 2 位十进制：最多 99 个出口；当前实际最多 13。
- `packed_id = table_code * 100 + exit_index`。
- 最坏情况约 34 个 segment/包装；当前 3282 个包装，输出 10,806,192 字节。

解释器解码在重建 exe 中直接可见：`0x1413362a6` 读取 `[rsi+0xc]`（parameter1，即 unary operator），`0` 走 `inc dword ptr [rbx+rax*4+0x640]`，`1` 走 `mov dword ptr [rbx+rax*4+0x640], r14d`（入口 `xor r14d,r14d`，所以是清零）；槽位索引由 `0x14132d350` 从 `[segment+8]`（function）解码。`20/21` 号槽在该路径使用固定偏移 `+0x690`/`+0x694`。也就是说，包装节点写的 `function=0x80+slot, p1=1`（清零）与 `p1=0`（++）正是解释器期望的原生编码；这比只看脚本格式更强，但仍不等于已在当前安装版本执行过。

包装的“头 + 返回 + 节点结束”形状也不是新发明的编码。原版 `em013_00.thk` 的 node4 有两个选项头（segment 14、26）：`function=0`、`marker=0x40`、`p1=15`（保留概率）、`local=5`，即“带权随机选项直接改调本地节点”。同一文件的 node5 是本地包装目标：segment 1 直接调用 `external_thk=55`，segment 11 `flow=8`（return），segment 12 `marker=1`（node end）且原始 `u32@36 = 10005`（节点索引 5 + 10000）。插桩器输出的头保留 `function=0`/`p1=weight`/marker，只把 `external_thk` 改成 `local`；包装体使用同样的 `external_thk=55` 调用、`flow=8` 返回、`marker=1` 且 `u32@36 = wrapper_node + 10000` 的结束段。

## 六、插桩范围与序列化

- 只要一张 random 表有**至少一个**可直接包装的正概率出口，就把它纳入目标；混合表只包装这些直接出口，本地调用/附加元数据出口保持原样（不产生事件）。
- 原版概率为 0 的出口不打标；后来被学习置零的原正概率出口仍保留标记以保持 ID 稳定。
- 当前覆盖 591/603 张表、3282/3328 个正概率选项。未覆盖的 46 个选项属于 22 张混合表的非直接出口（20 个）以及 12 张全部为非直接出口的外层表（26 个）；当前不支持把本地调用/非零元数据选项硬包装。
- 表号按排序后的目标集合分配；改变目标集合必须重新生成 Manifest、重建 Tracker，不能沿用旧映射。
- 导出会重建节点目录和数据偏移、追加包装节点，原节点索引和 ID 不变；原 segment 未改字段逐字节保留。末段原来缺省的 12 字节 padding 在追加数据时补齐。部署器按完整字节安装/更新，不依赖“仅权重差异”检查。

## 七、运行时接入、状态与部署时机

`src/register_mode.rs` 已接入旧 DLL 的启动路径：配置里出现 `register` 段时，`runtime::run_inner` 在完成部署器/看门狗/恢复后直接进入寄存器模式；没有该段时仍走旧的近似识别路线。

状态目录：`<游戏>/nativePC/plugins/FatalisAdaptive/register/`

- `learning.json`：唯一权威学习状态；启动时由它和原版基线重算权重，不信任模型文件。
- `model.thk`：当前权重的可读副本，便于检查，不参与启动恢复。
- `manifest.json`：本次插桩的 ID 映射、槽位、源/输出 SHA-256。
- `next.thk`：待安装的插桩字节（安装时由部署器复制）。

部署时机：标题/任务外稳定后安装并保持；任务结束稳定后先结算、重编译，再替换为下一场的插桩文件；**回到标题也不恢复原文件**，否则下一场会加载未插桩的 THK、寄存器事件为 0。只有 DLL/worker 退出（含看门狗进程退出恢复）或手动清理时才还原原文件；配置文件里关闭 `enabled` 时启动恢复逻辑会先还原。任务中启动插件不会热替换当前已加载的 AI，必须等下一次资源加载。

## 八、学习及一致性

同 ID 的 counter 增加开启新窗口。双方掉血取连续负 HP 差之和，治疗不抵销受伤，以开窗最大 HP 归一。任务最后只结算已经读到的部分。读取失败、非法 HP、未知 ID、counter 跳跃、最大 HP 改变等不把含糊伤害硬计给旧选项。

六个寄存器不是事务。清零/多次 ++ 写数字期间可能采到中间值；counter-before/数字/counter-after 双读只能排除部分并发情况。Tracker 在同 counter 下发现 `packed_id` 改变就丢窗以保守处理，未检测到的竞争仍可能存在。要实机测指令是否在同一 AI tick 连续完成；否则应通过经过验证的游戏线程采样点或额外提交协议消除竞争，不声称当前只读轮询严格无错配。

## 九、尚未完成，不能冒充可直接游玩

- **10.8 MB / 591 表 / 3282 选项的全量插桩 THK 尚未在游戏中加载**；上限、加载时间、内存和运行时性能都未知。
- 46 个非直接正概率选项未覆盖；它们可能包含特殊动作/本地控制，不应假装已全覆盖。
- 旧 DLL 的一次短局只验证了 node105 的 30 选项包链路；新全量包必须确认日志启动行、DLL 哈希、`events` 数量和 `failed_context`。
- 未验证清零/递增的实际调度、整数溢出，以及包装多一层调用对 reset/conclude 的影响。
- 未测试下一场资源刷新；快速重置/返回只做了保守结算。
- 派生链来自离线重建 exe 的静态证据；若日志持续出现 `failed_context`，改用显式地址，不要放宽校验。
