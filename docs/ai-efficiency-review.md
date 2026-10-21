# AI 提效审查与改进依据

日期：2026-10-21（Europe/Budapest）。源码基线：`3c6a730`，workspace 版本 `0.1.2`。

状态：**A 类 F01–F10 已完成修复与本地验收，准备发布 v0.1.3**。下文保留修复前的复现证据；B/C/I 类建议未实施。发布结果见 [修复计划](bug-fix-plan.md) 和 [v0.1.3 说明](releases/v0.1.3.md)。

## 判断标准与验证范围

DbGraph 的价值是让 AI 用更少调用和上下文，得到可核对的数据库事实。优先解决错误信心、无效重试、索引不可用，再增加数据库种类。

- 已阅读当前架构、任务包、能力矩阵和相关实现。
- `cargo test --workspace --offline`：135 个测试通过。其中 PostgreSQL live 测试没有配置 `DBG_TEST_DATABASE_URL`，提前返回，不能记为真实 PostgreSQL 验收。
- `cargo build -p dbgraph-cli --offline`：通过。冒烟使用本次构建的 CLI 与 MCP stdio。
- 冒烟只使用临时 SQLite 合成样例，没有连接业务数据库；所有运行发现均标记 **[冒烟发现]**。
- 未执行在线 LLM 对照、真实 PostgreSQL/MySQL/SQL Server 集成、平台安装验收或大规模延迟测量。

## 分类约定

按三类独立维护：A 是项目自身 bug；B 是外部项目已实现的优点；C 是外部 issue 报告的痛点与建议。每类分别给出本地证据、官方实现依据或 issue 状态。外部建议不自动成为本项目需求；最后的任务转化部分是本轮判断，不是外部项目的原话。

## A. 项目自身 bug

P1 表示应先修复的核心可靠性或提效问题，P2 表示较小边界缺陷。本轮未发现足以认定为 P0 的证据。

### F01 — P1 — SQL 引用漏检与错误消歧 [冒烟发现]

样例包含 `customers(id,email)`、`orders(id,customer_id,status)`；orders 没有 email 列。以下语句在 CLI 与 MCP 均返回 `valid: true`、`unresolved: []`：

```sql
select nonexistent from orders;
update orders set nonexistent = 1;
select * from orders where exists (select 1 from missing_table);
select * from orders o where o.email = 1;
```

前三项分别暴露 SELECT 输出列、UPDATE 赋值、EXISTS 子查询未被完整遍历。第四项提取到 `orders.email`，却用全 snapshot 中 customers.email 的末段名字错误地证明它存在。

影响：AI 会信任无效 SQL；列级影响、敏感字段访问与弃用字段使用也会漏报。加入 `select email from customers` 后，实际 analyze 只有 `risk.sensitive_column`，没有 `risk.query_reads_sensitive_column`。

源码：`crates/dbgraph-sql/src/lib.rs` 的 `visit_statement`、`visit_select`、`visit_expr`；CLI `commands/inspect.rs` 和 MCP `src/lib.rs` 的 `reference_exists` 与名称归一化。

改进：复用 AST，保留 schema/table/column、alias、查询作用域。不能通过截掉限定名消除错误；无法完整分析的语句显式报告 incomplete/unsupported。先覆盖已复现形态，不一次实现所有方言。

验收：反例不再通过，合法投影/赋值/子查询正常；跨表同名列、CTE、alias 遮蔽、带引号标识符结果明确；两入口共享判断。

### F02 — P1 — CLI 与 MCP 的 valid 含义不一致 [冒烟发现]

`select * from missing_table`：CLI 返回 `valid: true`、非空 unresolved、退出码 0；MCP 返回 `valid: false`。

CLI 当前把 valid 当语法解析结果，文本输出也标注 Parse；MCP 则综合语法与引用检查。这是接口契约不一致，不能把 CLI 字段解释成完整安全保证。

源码：`crates/dbgraph-cli/src/commands/inspect.rs:56`、`crates/dbgraph-mcp/src/lib.rs:901`。

改进与验收：统一语法状态、引用状态与整体判断，并明确退出码；同一 SQL/snapshot/dialect 结论一致；缺 snapshot、解析失败、引用不存在、分析不完整分别可辨识。

### F03 — P1 — 中文检索退化为无文本条件的列表 [冒烟发现]

给 `main.z_refunds` 配置描述“退款记录”，查询“退款”和“不存在的词”得到相同对象顺序，前列为 customers、orders、z_refunds。

源码：`crates/dbgraph-graph/src/search.rs:116` 只保留 ASCII 字母数字，空 terms 又允许全部对象进入结果。

改进：Unicode 词项保留、完整标识符匹配、空词项诊断，复用 semantic 描述；区分无匹配与明确列举对象。中文词法匹配不等于中英翻译，没有描述或项目词汇依据时应报告未匹配，不硬编码业务同义词。

验收：中文相关对象优先；无匹配不产生假答案；覆盖中英混合、短列名、空输入、过滤器；建议来自真实对象。

### F04 — P1 — 完整输出不受预算控制 [冒烟发现]

orders 的 semantic.allowedValues 包含 500 个合成值。`context --tokens 100` 返回 `estimatedTokens: 98`，但该结果重新序列化后约 7,900 字符；同条件的 MCP 整行响应为 15,145 字符，配置的 `mcp.maxResponseChars: 200` 未起作用。

源码：`context.rs:218` 主要累计名称和摘要，没有统一计算 semantic、路径、风险、建议、序列化开销。`max_response_chars` 只在配置定义/校验和 doctor 展示中消费，MCP 未执行它。

改进：预算约束完整内容，保留对象身份和关键证据，长数组按需展开；输出省略信息与具体后续参数。字符硬限制和 token 估算分别定义，过小预算明确失败，不能输出破损 JSON。

验收：长 semantic/SQL/描述、密集关系、小预算受控；省略计数与后续查询正确；粗估不宣称为实际模型计费值。

### F05 — P1 — 多文件相同 SQL 使 snapshot 失败 [冒烟发现]

`sql/a.sql`、`sql/b.sql` 都写 `select * from orders;`，snapshot 退出 1：

```text
SQLite storage error: UNIQUE constraint failed: objects.snapshot_id, objects.id
```

源码：`crates/dbgraph-sql/src/lib.rs:352` 仅以 fingerprint 生成 query/edge ID；snapshot 逐文件追加；storage 普通 INSERT。已有 artifact 去重测试未覆盖整条 snapshot 流程。

改进：分开文件来源身份与 workload fingerprint。保留各文件定位信息，fingerprint 用于聚合，不以覆盖 INSERT 或丢弃文件隐藏冲突。

验收：多文件相同 SQL 可建图且来源都在；工作负载不重复计数；移动/删除文件的 diff 有明确行为。

### F06 — P1 — 建图失败后仍发布未完成的 latest [冒烟发现]

先正常 snapshot，保留索引，再用 F05 触发失败。新 JSON 仍成为 latest，含 16 个对象但仅 15 个唯一 ID；旧 snapshot 仍在索引中，新 ID 不在；`search orders` 却退出 0，读取失败流程留下的新 JSON。

源码：`commands/snapshot.rs:147` 先写正式 JSON，再建索引、写 artifacts；SnapshotStore 按文件名找 latest；require_graph_index 只检查文件存在。

改进：JSON 继续是事实源，但只发布完成的 capture；暂存文件不参与 latest 发现，索引作为可重建派生数据。定义清晰的完成点，不增加第二套事实源或默认回退路径。

验收：索引/artifact 写入失败不发布未完成状态；旧成功版本可用；重试可成功，查询不混用版本。

### F07 — P2 — 索引缺失时 sync 仍跳过恢复 [冒烟发现]

对无 SQL 文件的正常样例，仅删除临时 `.dbgraph/dbgraph.db`，保留 JSON。sync 退出 0，plan=unchanged、snapshot=null；索引仍不存在，之后 search 退出 3。

源码：`commands/snapshot.rs:195` 只根据 schema hash 决定跳过重建。

改进与验收：区分事实是否变化与派生索引是否可用；健康无变化跳过，缺索引从有效 snapshot 恢复，失败明确报错，不制造伪 schema 变化。不引入昂贵全文件 hash 或后台守护来掩盖缺口。

### F08 — P2 — 一个快照时 diff 比较自身 [冒烟发现]

仅一个 snapshot，diff 成功且 previousSnapshotId=latestSnapshotId、changes=[]，实际没有历史基线。

源码：`crates/dbgraph-core/src/snapshot.rs:110` 的 `len().saturating_sub(2)` 在长度 1 时取到当前文件。

验收：零/一/二/多 snapshot 分别覆盖；一个时明确缺少基线，两个时比较正确版本。

### F09 — P1 — 子查询 WHERE 掩盖外层无限制 UPDATE [冒烟发现]

以下 SQL 无外层 WHERE，但实际 analyze 没有 `risk.update_without_where`：

```sql
update orders
set status = (select email from customers where id = 1);
```

只做解析与分析，未执行该 UPDATE。源码：`crates/dbgraph-graph/src/analysis/mod.rs:647` 使用字符串 `contains(" WHERE ")`。

改进：复用 F01 的 AST 结果检查顶层 selection，不继续增加字符串补丁。

验收：无外层 WHERE、仅内层 WHERE、多语句、字符串中的 WHERE、CTE 包裹 DML 各有结果；合法外层条件不误报。

### F10 — P2 — 发布平台文档与 workflow 不一致 [源码确认，未做发布冒烟]

`docs/release-matrix.md` 列出 macOS x64；`.github/workflows/release.yml` 当前只列 macOS ARM64。未核实线上已有资产，不能据此断言所有历史 macOS x64 包缺失。

改进与验收：按实际支持范围统一文档、安装器和构建矩阵；若保留该平台承诺，验证对应资产与安装，否则明确缩减支持范围。

## v0.1.3 修复结果与新增验证记录

| 缺陷 | 修复和验收结果 |
| --- | --- |
| F01/F02 | 共享作用域分析；投影/赋值/子查询、alias 隐藏原名、UPDATE FROM 目标隔离、派生表同名列歧义测试通过。CLI/MCP 均要求本地 snapshot/index；5 个错误 SQL 均 false、CLI 退出 2；3 个合法对照均 true。不支持的分析明确诊断。 |
| F03 | Unicode 与短标识符保留；“退款”命中 main.z_refunds，“不存在的词”返回空。过滤器、空输入与无匹配边界通过。 |
| F04 | 完整 pretty JSON 计量；超大对象跳过并继续尝试小对象，不裁断 JSON。实际 800-token 预算结果 estimatedTokens=590，100-token 请求明确失败；MCP 200 字符配置返回 45 字符完整 response_budget_exceeded 错误。丰富展开/后续建议留作后续候选。 |
| F05 | query/edge 用相对路径，artifact 用 snapshot+来源；两个相同 SQL 来源保留，加风险文件共三个 artifact 正常 snapshot。历史 artifact/workload 去重回归通过；未新增 hash 校验。 |
| F06 | 暂存 JSON、图/artifact 同事务、最后原子发布。实际注入 artifact INSERT 失败，旧 latest 和 artifacts 保留；移除故障后重试成功。未测试断电/进程崩溃。 |
| F07 | 缺失/空索引恢复；实际 sync 返回 indexRebuilt=true、snapshot=null，保留一个 JSON，search 恢复。sync 仍先采集 provider，需要数据库可访问。 |
| F08 | 零/一快照无 previous，两个比较不同 ID；实际单快照 diff 退出 3，重试产生两个快照后 diff 正常。 |
| F09 | AST 外层 selection 判断；实际 analyze 同时检出 risk.update_without_where 与 risk.query_reads_sensitive_column。只解析风险 SQL，没有执行。 |
| F10 | 补 macos-15-intel，workflow/installer/docs 六平台一致。远端 v0.1.2 实际为五个平台二进制；v0.1.3 runner/资产结果发布后写回计划，不能替代原生安装验收。 |

- **[源码复核发现，已修复]** 根 npm package 为 0.1.2，嵌套 launcher package 仍为 0.1.0，而 launcher 实际读取嵌套版本。现统一为 0.1.3，避免新版 wrapper 下载旧二进制。
- **[官网冒烟发现，已修复]** 触碰页面的正文 `.md` 链接与缺失 favicon 导致 404；修复后桌面导航、手机中文页通过，详见修复计划。
- **[验证环境问题，已解决]** D 盘构建缓存耗尽空间，使用 cargo clean 清理本项目 11.9 GiB 构建产物，并关闭本地调试符号/增量缓存完成验证，没有改产品逻辑。
- **[测试夹具问题，已修复]** 旧 snapshot 测试用毫秒时间戳创建目录，单测独立通过、并行时互删导致 PermissionDenied。改用已有 tempfile 库，没有串行掩盖竞态。
- **[验证脚本问题，已纠正]** 首次最终冒烟把 CLI search 的 `{query, results}` 报告误当数组而 KeyError；按实际合同读取 results 后通过。Windows GBK/UTF-8 读取差异也仅修正脚本编码。
- 合并后 workspace **152 项测试通过**；其中 live PostgreSQL 用例无连接配置而提前返回，不能记为真实服务通过。fmt、全 target clippy（`-D warnings`）、CLI build 通过。隔离 SQLite CLI/MCP 冒烟通过；临时记录 `dbgraph-v013-smoke-bokq0lc2/smoke-result.json`。
- 未新增 hash 校验、性能硬门槛、精确评分断言或在线 LLM 调用；未实施 B/C/I 类候选功能。

## B. 其他项目已实现的优点

以下依据官方文档/实现说明，不把 issue 提案当成已实现能力，也不把对方自报数字作为我们的收益承诺。

| 编号 | 项目与官方依据 | 已有优点 | 值得借鉴的部分 | 适配成本与边界 |
| --- | --- | --- | --- | --- |
| B01 | [DBHub：MCP tools](https://github.com/bytebase/dbhub#mcp-tools) | search_objects 通过逐层展开获取 schema/table/column 等元数据 | 首次返回任务所需摘要，详细信息按需查；跟进参数可直接使用 | 中；复用现有 search/table/context，不另造查询系统，不引入其 execute_sql |
| B02 | [DBHub：Token Efficiency](https://github.com/bytebase/dbhub#token-efficiency) | 默认工具集较小，可选能力按需开启 | 评测工具定义与每次结果的总上下文成本，减少重复描述与冗余 payload | 低到中；不能仅为减少工具数合并不相干职责；不直接采信其倍数比较 |
| B03 | [SQLGlot：AST primer](https://github.com/tobymao/sqlglot/blob/main/posts/ast_primer.md) | 用 scope 和列限定解析来源，必要时利用 schema 消歧 | 将语法成功、名称解析、引用覆盖分开，形成共享分析结果 | 中到高；保留 Rust/sqlparser，借鉴解析结构，不为此引入 Python 服务或完整优化器 |

## C. 其他项目 issue 提出的痛点与建议

页面于本轮检索。状态按页面所示；已关闭问题作为历史经验，不写成外部项目当前仍有缺陷。报告者的观测不是本轮复现实验。

| 编号 | Issue 与状态 | 报告的痛点/建议 | 我们是否也有证据 | 本项目转化建议与优先级 |
| --- | --- | --- | --- | --- |
| C01 | [CodeGraph #1372](https://github.com/colbymchenry/codegraph/issues/1372)，已关闭 | 中文文件名查询无结果 | 有：F03 的本地中文查询退化，但返回形态与对方不同 | P1，修词项提取并测试中文/中英混合；不把 Unicode 保留夸大成语义翻译 |
| C02 | [CodeGraph #1904](https://github.com/colbymchenry/codegraph/issues/1904)，已关闭；维护者说明已在 main 修复 | 查询词不在索引中时，只返回空结果，用户无法区分没有该概念和用词不符；建议给词项诊断与真实候选 | 本项目 search/context 缺少这样的诊断；不是证明不存在该业务概念 | P2，先修 F03，再加有依据的未匹配原因/具体后续查询，避免让 AI 回到无效反复搜索 |
| C03 | [CodeGraph #1520](https://github.com/colbymchenry/codegraph/issues/1520)，开放 | camelCase 子词召回、字段权重、过滤与去重质量不足 | 本项目有不同的 snapshot 扫描路径，未复现对方 FTS 的具体比例与全部问题 | P2，纳入标识符/重复对象检索基准；测后决定接入已有 FTS，不照搬引擎细节 |
| C04 | [SQLGlot #5112](https://github.com/tobymao/sqlglot/issues/5112)，已关闭 | 大型真实查询做优化和血缘分析较慢 | 尚未测量本项目复杂 SQL 的同类延迟 | P2，增加深嵌套/大查询成本基准；共享解析结果，不为提取引用执行全部优化流程 |

## 转化为本项目任务的判断与取舍

以下是根据 A/B/C 和当前源码作出的实施建议，不是新增第四类外部事实。B/C 并非全部照搬；新架构或依赖必须由本项目实际问题与测量支持。

### I01 — 用真实任务指标验证收益

`benchmark-agent` 仅支持 teashop；baseline 拼接原始材料与完整 snapshot，DbGraph 分支依据 `expected_objects` 筛选 findings，并固定计为 2 次检索。见 `crates/dbgraph-cli/src/commands/benchmark_agent.rs:98`。

现有实验明确声明为离线估算；不能证明真实 Agent 节省 89.6% 费用。期望答案参与待评上下文筛选，若用来评价端到端检索则存在答案信息泄漏。

下一步建立小型任务集：SQL 修错、字段影响、敏感字段定位、中文搜索、无答案查询、索引恢复。期望证据只用于评分；检索端只看到问题和工具结果。两组用同样资料、工具边界、停止条件。

指标：任务正确率、错误自信/漏报、实际工具调用、重试、累计响应字节/估算 token、冷/热延迟。模型对照另定固定模型、提示、费用和重复次数，本轮未调用模型服务。

### I02 — 测量后使用已有索引优化

SQLite storage 已有 FTS；CLI/MCP 主要 search/context 仍读取完整 JSON，调用 search_snapshot。context 邻居扩展还会反复线性查找对象 ID。不能写成“尚无 FTS”。

`benchmark` 目前生成 synthetic schema 并报告对象/边数与 hash，没有测建图/search/context 耗时。10k 对象测试通过不等于交互延迟达标。

先测 1k/10k 表、不同 SQL 数量与邻接密度的实际路径、内存、响应大小，再对瓶颈使用已有 FTS、每请求对象映射或有限候选。保持统一排序，避免 CLI/MCP 各自实现，不无依据增加缓存或向量服务。

### I03 — 可追溯且能继续查询的上下文

context 已有对象、semantic、路径和建议，但缺少随内容返回的 snapshot 标识、采集时间、匹配依据与覆盖缺口。SQL sourcePath 已在 metadata，优先复用，不建平行出处账本。

首屏给足任务相关对象/必要关系/出处，大枚举和详细 profile 按需查。可报告快照年龄；未连接实时数据库时，不能声称实时 schema 已确认未变。

用 I01 验收：减少补查而关键证据召回不降；无匹配、未覆盖、预算省略、旧快照各有明确表达。

### I04 — 交付检查与平台文档

CI 和 postgres-integration 仅在 v* 标签触发，release workflow 独立构建上传。提交 `6c6e8cf` 明确把 PR/push 改成 tag-only，不能当作无意遗漏直接恢复。

建议明确成本策略后，将轻量检查前移或设置手动预发布检查，Postgres 按相关路径/按需运行，发布等待必要验证。本轮未改 workflow。

发布平台文档与 workflow 的已确认不一致独立记录为 F10。

### I05 — 按需求扩 provider

本轮确认 MySQL/SQL Server 仍是 unsupported.rs 占位，Postgres/SQLite 已实现。新 provider 复用统一 snapshot，完成真实服务的权限、catalog 和边界验收。仅在明确用户被缺少 provider 阻塞时提前，否则先修共同链路。

## 实施顺序、开销与停点

| 顺序 | 切片 | 相对开销 | 收益与完成条件 |
| --- | --- | --- | --- |
| 1 | F01/F02/F09，共享 SQL 分析与一致结论 | 中到高 | 减少错误信心；反例、合法对照、作用域边界通过，未覆盖明确 |
| 2 | F05–F08，来源身份、发布与恢复 | 中；F07/F08 较小 | 降低首用失败与手工修复；重复来源、失败发布、缺索引、单快照均通过 |
| 3 | F03/F04 与 I03 最小输出 | 中 | 减少补查与上下文浪费；检索准确、预算有效、有出处和后续参数 |
| 4 | I01/I02，任务与性能评测 | 中 | 固定任务集无答案泄漏，输出正确率/调用/体积/延迟，再决定优化 |
| 5 | I04 检查策略、I05 按需 provider | 依成本与需求 | 检查策略明确；provider 必须有真实服务验收 |

各切片通过验收即结束，不以功能数量或评审次数作为完成标准。实施阶段发现新问题按相同文档归属记录。

暂不优先：云端索引、向量库、完整 Web 管理台、模型直接写业务 SQL、一次补齐全部方言。没有发布、业务数据导入或生产配置变更授权。

## 复现环境与步骤

临时目录：`C:\Users\zaenz\AppData\Local\Temp\dbgraph-audit-20261021-101xywo2`。可能被系统清理，不作为永久 fixture。

1. 新临时目录执行 `dbgraph init <dir>`，用 Python sqlite3 创建独立 business.sqlite。
2. 建表：customers(id INTEGER PRIMARY KEY,email TEXT)、orders(id INTEGER PRIMARY KEY,customer_id INTEGER,status TEXT)、z_refunds(id INTEGER PRIMARY KEY,order_id INTEGER)。
3. 临时配置 database 指向 sqlite 绝对路径，移除 connectionEnv；保留默认 schema profile，mcp.maxResponseChars=200。
4. semantics 设置 main.z_refunds.description=退款记录；main.orders.description=Orders，allowedValues 为 state_0 到 state_499。
5. snapshot 后执行本文 validate-sql/search/context；MCP 用 newline JSON-RPC tools/call，projectPath 指向该目录。
6. 第一次 snapshot 后执行 diff；仅删除该临时项目生成的索引，再 sync/search，验证 F07/F08。
7. 两个 SQL 文件内容相同验证 F05；保留正常索引后再加入重复文件验证 F06。对比 SQLite snapshots 表与 latest JSON ID。
8. 只保留一个 SQL 文件，写入 F09 与敏感字段 SELECT，再 snapshot/analyze。

辅助脚本曾因 Windows Python 默认按 GBK 读中文 JSON 失败，随后显式 UTF-8 完成验证；这是审查脚本问题，不计作产品缺陷。中文查询另用 Unicode 转义复核，排除了终端显示编码影响。
