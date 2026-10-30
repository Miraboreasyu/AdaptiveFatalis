# 外置学习策略（0.4 测试版）

## 给玩家

安装包覆盖 DLL、EXE、配置和 policies/default.rhai。0.3 的 register/learning.json 可保留，默认第一次建立 profile 时导入其中的累计统计；之后外置策略使用自己的 profile，不再更新旧 learning.json。不要删除 deployment，它负责恢复原 AI。

config.json 新字段示例：

```json
"scripted_learning": {
  "script": "policies/default.rhai",
  "profile": "default-v1",
  "params": {"direction": 1.0, "strength": 2.0, "prior_samples": 8.0, "exploration": 0.25},
  "import_legacy": true,
  "debug": false
}
```

默认脚本 direction=1.0 恶意、-1.0 善意、0.0 返回基线。切换同一脚本方向可以保留 profile。改脚本状态结构、奖励定义或换算法时用新的 profile，并按需要关闭 import_legacy。没有任何脚本内容哈希；修改脚本时由作者负责状态兼容。

只有启动、战后结算运行脚本，AST 每次进程启动只编译一次。不在每帧执行脚本，也不扫描全量历史。重启读取 state 快照并从字节游标续读新增事件。没有新增策略 SHA256、签名或每帧校验。

## 算法作者接口

Rhai 导出两个函数：

```
fn update(state, events, params) { /* return new object state */ }
fn weights(state, table, params) { /* return float array */ }
```

update 按一次结算批次调用。events 是数组，每项包括 branch_id、table_id、exit_index、player_loss_fraction、monster_loss_fraction、started_at、ended_at、counter 和 player_fainted（当前未知，null）。损失是窗口内负 HP 差累加再按开窗最大 HP 归一化，可能超过1；观测层不截断。默认算法自行截断1以兼容旧行为。

当前事件接口使用归一化损失，不提供精确命中来源或原始HP；窗口可能覆盖后续未插桩动作，不能把它当命中次数或单招绝对因果证明。猫车检测尚未实现，不能通过未知字段假造击杀奖励。

weights 的 table 包含 id、branches 数组，每项 id 和 base_weight。同一 Global 在不同表内保持独立。输出必须是 float 数组，长度一致、有限且非负；原始0选项必须仍为0，合法总和必须正。宿主归一化至100并按最大余数法整数化。策略可以把原来正概率选项设0；宿主不会强制探索。默认 exploration 参数保留探索。

脚本不能读写文件、网络、进程或游戏内存，import/eval禁用，执行步数/深度/容器有上限。不要在顶层写耗时逻辑。状态是 JSON 可序列化对象。weights 应是纯函数：只从事实状态算权重，不在此学习或依赖墙钟/随机数；宿主传入的是隔离副本。

## 状态、回放和日志

- register/events.jsonl：每次非空结算追加一行，包含本场完整事件批次。
- register/profiles/<profile>.json：state、hunts、cursor。cursor 是已成功处理日志的字节位置，不是内容校验和。
- runtime.log：保留现有采样诊断，添加策略加载、失败、处理窗口数、概率变化与编译部署结果。变化列表相对原始基线，不是相对上一场。
- debug=true：另输出 <profile>-weights.json，包含有变化表的浮点目标和整数结果。不逐帧刷日志。

先落事件，脚本在副本上更新，所有表输出通过后提交 checkpoint。脚本失败不推进游标、不覆盖状态；保留旧模型与日志，修脚本重启重试。新启动脚本加载失败会停止本次自适应部署并由原有保护恢复目标，不会悄悄使用别的算法。日志尾部若因断电截断会显式报错，保留备份后修复不完整末行；不盲目吞掉数据。

切换参数后重新启动会以已有 state 重算。改变影响 update 的参数不会追溯改变旧 state；要追溯就换新 profile 并从日志回放。没有旧逐事件日志的累计历史不能无损重建；import_legacy只支持理解旧 effects 结构的算法。自动压缩/轮转日志暂不做，避免破坏游标；长期可备份日志并建立新 profile。

## 离线重算

高级用户可用 fatalis-ai.exe 的 policy-replay MAIN GLOBAL PRIVATE_DIR NEW_OUTPUT。务必复制整个私有状态目录到工作区再跑；此命令会推进该副本的 profile，不碰游戏进程、不部署THK。NEW_OUTPUT 必须不存在。删掉副本中目标 profile 文件可从该副本事件日志重新回放；若要只用新事件，关闭 import_legacy。

0.3 插桩与跨场部署已有两场实机日志。0.4 Rhai/日志游标/状态恢复当前只有离线验证，须再次实机测试。安装包不包含加载器。
