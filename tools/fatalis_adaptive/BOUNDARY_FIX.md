# 0.4.1 采样边界修复

最大 HP 改变、怪物 token 改变、寄存器计数重置时，结算至最后一个可靠快照，不再删除整个活动窗口。变化后的快照不参与旧窗口伤害计算；同计数的事件不会因为恢复读取而重开。时间回退、未知选项、不一致 ID、丢计数和无效读取的保守丢弃规则保持不变。

runtime.log 的 `register boundary: BoundaryPreserved` 不受五秒异常限流影响，记录原因、表、选项、counter、观测时间范围、累计归一化损失、起始快照、最后已观测 HP（observed_damage）及变化快照。并非猫车检测，也不补造未采到的致死伤害。

super_malevolent.rhai 改为单窗口 player_loss_fraction 严格 >0.9 锁定所在表为100/0；恰好0.9不触发。沿用 super-malevolent-v1 状态，已处理的历史日志不会因改阈值自动重放，历史已丢失窗口无法恢复。

验证：完整 all-features 测试69单元+28集成通过，clippy -D warnings通过。最后增加诊断起始快照、加强token/reset断言后，重新运行10个tracker测试、clippy与release构建均通过。脚本测试覆盖0.9不触发、0.900001触发、多次小伤害不相加、最后达标者覆盖、跨表隔离。打包验证外置脚本加载、无事件基线不变及zip完整性。

包：fatalis-adaptive-0.4.1-super-malevolent-fix-test-win64.zip。必须游戏关闭后同时覆盖DLL/EXE、配置和脚本。保留register与deployment，备份自定义配置。本轮未修改游戏安装目录；修复尚待实机确认。
