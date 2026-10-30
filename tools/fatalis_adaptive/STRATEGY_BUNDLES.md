# 0.6.0 策略包与最小主配置

主config.json只允许 strategy：none（默认内置无学习）、standard-malevolent、standard-benevolent、fast-malevolent、fast-benevolent。旧大配置不再接受，升级必须覆盖配置；原始历史和部署恢复记录保留。

策略目录 strategies/<选择名>/ 内包含 strategy.json、algorithm.rhai、params.json、README.txt。描述文件字段 id/state_version/script/params；参数是任意JSON对象，含义由脚本作者解释。算法只通过宿主传入的params读取它，不直接读文件。

存档自动命名 id-v<state_version>；新存档启动时回放历史事件，已有存档继续增量处理。修改脚本state结构必须提高state_version。参数变化本身不重置state。善意与恶意的自带包使用不同id，互不污染。策略包作者可以明确复用id，但必须保证状态语义兼容。

寄存器槽14..19、全表目标、地址派生链、采样周期16ms、稳定时间1500ms均在内部运行配置固定，不出现在玩家配置。旧技术Config类型保留给测试与内部适配，不作为玩家文件格式。none无需外部文件。

策略名只允许字母数字下划线与连字符；包目录和成员经规范路径校验不得逃逸目录；参数必须对象，错误不静默改用其它算法。

69单元+30集成测试通过，clippy通过，release重新构建。包解压后分别执行none和4个策略的离线回放，均成功并在无事件时输出原版概率。新策略包解析验证固定协议、参数方向、存档命名、非法策略名、缺失策略和旧参数拒绝。

已确认游戏关闭，备份并更新游戏安装目录的插件、最小config和4个策略包；register、deployment、baseline和旧policies均保留。旧policies不再被新选择接口使用。默认none，无需选择学习算法。实机启动尚未验证。
