# 项目自身 bug 修复计划

日期：2026-10-21。范围只包含 [审查 A 类](ai-efficiency-review.md#A-项目自身-bug) F01–F10。B/C 类、I01–I05 的能力扩展不实施。状态：三轮计划审查及实施后独立复核完成，F01–F10 本地验收通过，准备发布 v0.1.3。

## 执行次序

| 批次 | 缺陷 | 实施与责任边界 | 必要验收 |
| --- | --- | --- | --- |
| 1 | F01/F02/F09，P1 | dbgraph-sql 统一 AST 引用解析和顶层 DML 条件；CLI/MCP 消费同一结论；graph 风险规则使用 AST 结果 | 审查中的错误 SQL 不再通过，合法对照正常；alias/CTE/子查询、同名列、解析失败；CLI/MCP 一致 |
| 2 | F05/F06，P1；关联 F07/F08，P2 | SQL 文件身份与 fingerprint 分离；索引/artifact 同一 SQLite 事务；JSON 暂存后发布；sync 恢复缺失索引；修正前一个快照选择 | 重复 SQL 来源保留、失败不替换 latest、恢复可用、一个快照不比较自身 |
| 3 | F03/F04，P1 | 词项保留 Unicode 和短标识符；完整 context 计量；MCP 执行 maxResponseChars | 中文/无匹配/过滤器正常；长 semantic 不虚报预算；小预算给明确结果且 JSON 完整 |
| 4 | F10，P2 | 按安装器与 release_targets 已有的六平台合同，补 macOS Intel 构建项；不改 CI 触发策略 | 本地矩阵一致性检查；真实 macOS 构建/安装留待对应 runner，不能标成本地通过 |

## 明确的实现选择

- 继续使用 Rust/sqlparser/SQLite。SQL 的未知语句/引用不能静默变成验证成功；不增加兼容判断、全方言优化器或其他项目功能。
- 保留完整限定名与查询作用域，不通过全库同名列匹配消除 unresolved。CLI 无效 SQL 输出报告后返回非零退出码。
- query 身份使用相对源路径，fingerprint 只负责 workload 聚合；artifact 身份包含 snapshot 与来源。storage 中错误的 fingerprint 唯一索引需要最小数据库结构升级，这是数据库改动，不是旧配置兼容层。
- JSON 是已发布事实源。先暂存 JSON，再完成索引/artifact 事务，最后发布 JSON；失败前的有效 latest 不变。事务已完成但发布失败的未发布索引不得被当成 latest。无需跨文件事务框架、第二份状态账本或自动 fallback。
- context 预算统计完整 JSON 内容；MCP 字符预算约束工具结果文本，不把传输层 JSON-RPC 包装计为可控业务内容。预算无法容纳内容时返回可识别错误，不截断 JSON。
- F10 沿用既有六平台支持承诺；`macos-15-intel` 标签已经按 [GitHub 官方 runner 文档](https://docs.github.com/en/actions/reference/runners/github-hosted-runners) 核实。

## 三轮计划审查记录

计划阶段由主代理分阶段审查，当时未使用独立审查者。实施中用户允许 subagent 后另行进行 SQL 和存储独立复核，不混为计划阶段的三轮审查。

1. **事实与范围**：对照已有九项冒烟、一项源码发现以及当前源代码。剔除 I 类评测、FTS 接入、可追溯新字段、工具集重组、provider 扩展等建议。本轮只修既有 bug。
2. **架构与成本**：发现 F05 不仅是 object ID 冲突，storage 也有 fingerprint 唯一索引；纳入最小数据库升级。F06 必须连同 artifact 事务处理，不能只改写文件顺序。F10 的安装脚本和 Rust 矩阵都承诺 x64，选择补构建项而不是无依据缩减支持。SQL 分析留在现有 owner，不在 CLI/MCP 各补一套。
3. **失败与验收**：补上失败发布、缺失索引、无历史基线、无匹配输入、过小预算和 CLI/MCP 一致性。按用户最新要求收紧测试成本：仅直接复现、正常对照和必要边界；不增加 hash 校验、精确计分/输出快照断言、性能硬门槛或全方言穷举。平台运行环境不具备时明确未验证，不借本地测试代替。

三轮未发现需要扩大授权范围的问题。实施发现新事实时只复审受影响决定，不重复整个计划。

## 检查与文档

- 每批运行受影响测试；相关修改合并后执行 workspace tests、fmt、clippy 和 build。实际失败才追加检查。
- 最终用隔离 SQLite CLI/MCP 复测关键路径，不连接业务数据库、不执行风险 SQL。
- 在原审查 A 类更新状态和证据，保留“冒烟发现”标记；B/C 内容保持原样。同步使用文档中变更的 CLI 退出码、预算和恢复操作合同。
- 不新增 hash 校验测试或校验流程；不扩展现有产品 fingerprint/schema change 机制。
- 用户追加授权：修复与检查完成后更新文档、官网并发布 patch release。沿用现有 GitHub release 与 Pages 工作流；核对版本及发布结果。云端新任务暂缓，本次继续本地完成。所有尚未完成的边界在交付中明确列出。

## 文档、官网与发布矩阵检查

- F10 的 workflow、Rust installer `release_targets`、文档均包含相同的六个 target；macOS x64 明确使用 `macos-15-intel`。本地只检查矩阵内容，不能代替对应平台构建和安装。
- 官网已同步英中使用指南、配置、发布矩阵及首页可靠性说明。`npm run build` 生成 17 个页面，退出码 `0`。
- 本地预览 `http://127.0.0.1:4327/dbgraph/`：Edge 桌面 `1440×1000` 验证首页 → usage → configuration 导航及发布矩阵；`390×844` 检查中文使用页。页面身份、正文、错误遮罩、console、链接和横向溢出检查通过；最终 console 错误/警告为 `0`。证据保存在临时目录 `dbgraph-site-qa-20261021`，不提交截图或测试脚本。
- **[官网冒烟发现，已修复]** 原正文的 `configuration.md` 相对链接会请求 `/dbgraph/usage/configuration.md`，实际返回 `404`。本次触碰页面改为 `/dbgraph/.../` 站点路由，并验证首页 → usage → configuration 导航。
- **[官网冒烟发现，已修复]** 默认请求 `/dbgraph/favicon.svg`，仓库没有该资产，浏览器 console 报 `404`。补充小型 SVG 图标，不变更页面布局。
- **[构建提示，保留边界]** 现有 Astro/Starlight 集成输出 markdown 插件 API 弃用提示及默认 `404` 内容项提示，构建仍成功；本轮不升级依赖。页面验证与这些构建提示分开记录。
- **[验证环境/脚本问题，非产品缺陷]** 捆绑 Playwright 默认 Chromium revision 未安装；使用已安装 Edge 完成页面检查，未下载新浏览器。初始定位器同时匹配正文链接与底部导航，改用可见链接名后继续；没有为通过测试修改产品行为。
- `sync` 仍先连接 provider 采集元数据，再判断 schema 是否变化；缺失索引恢复不意味着离线可用。该边界已写入英中使用指南。
- `v0.1.3` release notes 已准备，发布状态仍待主代理最终检查和线上结果。

## 合并验收与发布

- workspace 152 tests、fmt、全 target clippy（`-D warnings`）、CLI build 通过。PostgreSQL live 用例没有连接配置，不能记为真实服务通过。
- 真实 CLI/MCP：5 个错误 SQL 结论一致、3 个合法对照通过；投影敏感字段/外层无条件更新正确报告；中文匹配与无答案、完整 context 预算、MCP 200 字符限制通过。
- 重复 SQL 来源 snapshot 成功；缺索引 sync 恢复；artifact 写入故障保留旧 latest，清除故障后 snapshot/diff 成功。未开展断电、跨平台原生运行或大规模延迟测试。
- 独立 SQL 复核修正三处作用域边界；存储只读复核没有阻断项，明确 sync 仍需 provider 可访问。预算计算经性能复核复用一次对象名称映射，未新增常驻缓存。
- 版本统一为 v0.1.3，包含此前落后的嵌套 npm launcher package。准备推送现有发布工作流并手动部署 Pages，线上结果完成后写回此处。
