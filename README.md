# AdaptiveFatalis

怪物猎人：世界 / 冰原的实验性黑龙跨战斗自适应 AI。通过 THK 选项插桩与寄存器读取记录分支选择，统计事件窗口内玩家/怪物 HP 损失，由外置 Rhai 策略生成下一次资源加载使用的概率。

## 当前成果

- 591 张表、3282 个原始正权重直接选项插桩；603 张随机表中另有46个非普通控制选项不支持插桩，不能称为所有 AI 路径全覆盖。
- 跨场事件日志、策略状态、生成 THK 与安全部署/退出恢复。
- 默认 `none` 无学习，保留事件采集但输出原版概率。
- 自带标准/快速、善意/恶意四个独立策略包。
- 主配置只选择策略；参数、脚本、状态版本随策略包提供。
- 最大 HP/怪物实例/计数重置时保存可靠观测前缀，不把重生 HP 计为伤害。
- 部署器按需创建 THK 目录，不需要压缩包包含空 em 目录。

用户已在本地实机验证事件采集、跨场学习与资源加载；极端测试策略成功将扇火所属表锁为100/0；快速善意策略已实际生成多张表的概率变化。**这不是通用版本兼容保证**：当前偏移、原始资源哈希和插桩模型针对本项目验证过的黑龙资源，其他构建可能不适用。

## 构建（Windows x64 / Rust）

从仓库根目录运行：

    cargo build --release --locked --features offline-cli --manifest-path tools/fatalis_adaptive/Cargo.toml
    cargo test --locked --features offline-cli --manifest-path tools/fatalis_adaptive/Cargo.toml
    cargo clippy --locked --features offline-cli --all-targets --manifest-path tools/fatalis_adaptive/Cargo.toml -- -D warnings

产物位于 `tools/fatalis_adaptive/target/release/`：DLL、部署/恢复辅助 EXE、离线诊断 EXE。需要已有的兼容 nativePC 插件加载器；本仓库不提供加载器。

## 配置

插件私有目录为 `nativePC/plugins/FatalisAdaptive/`。主 `config.json`：

    { "strategy": "none" }

可选 `standard-malevolent`、`standard-benevolent`、`fast-malevolent`、`fast-benevolent`。将源码中的 `strategies/` 目录复制到私有目录。算法包内 `params.json` 控制算法参数，`strategy.json` 声明 `id`、`state_version`、脚本和参数文件。

新的策略状态在启动时回放已有事件；同一状态继续增量学习。更改不兼容state结构应提高state_version。策略只在启动/结算运行，不在采样循环执行。

## 游戏资源不随仓库分发

仓库不包含原版/生成 THK、游戏可执行文件、玩家战斗日志、学习状态、构建缓存或本地测试压缩包。

要实际运行，需自行合法提取匹配的 `em013_00.thk` 和 `em013_55.thk`，使用辅助工具初始化私有目录：

    fatalis-adaptive-tool.exe init PRIVATE_DIR MAIN_THK GLOBAL_THK

DLL与辅助EXE放在 `nativePC/plugins/`，基线位于私有目录的 `baseline/`。不要手动运行内部watch命令；辅助程序由插件自动启动负责进程退出恢复。游戏关闭后升级，保留 `register/` 和 `deployment/`，切勿盲目删除部署恢复记录。与其它黑龙THK覆盖mod冲突时插件拒绝覆盖。

## 测试

默认测试不需要游戏资源。原资源集成测试通过 `game-fixtures` 功能显式启用：将匹配的60份 `em013_*.thk` 私下放入仓库根 `research/fatalis_thk/`，再运行 `cargo test --features offline-cli,game-fixtures ...`。这些资源被gitignore排除，**不要提交游戏资源**。因此在没有私有fixtures的干净克隆里不要直接 `--all-features`。

本项目开发过程中完整资源测试已通过；公开仓库另做无游戏资源的测试/构建验证。旧专项技术文档保留研发历史，其中旧配置与旧测试包说明已被当前最小配置和策略包机制取代。

## 限制

HP窗口归因不是精确伤害来源追踪：破头倒地追打、延迟攻击和窗口内其它伤害可能计入同一选项。没有可靠猫车事件，脚本中的player_fainted当前为null。单个快照可能漏掉致死伤害，不能由插件虚构。Rhai虽然受操作数/输入大小限制，策略作者仍应避免保存无界状态。请备份mod并优先单人测试。

技术说明见 `tools/fatalis_adaptive/STRATEGY_BUNDLES.md`、`SCRIPT_POLICY.md`、`REGISTER_INSTRUMENTATION.md`。
