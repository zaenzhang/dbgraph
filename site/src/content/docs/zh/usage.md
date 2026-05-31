---
title: 中文使用说明
description: DbGraph 中文完整使用流程、配置、常用命令和安全说明。
---

# DbGraph 使用说明

DbGraph 是一个本地优先的数据库上下文工具。典型使用流程是：

1. 初始化项目
2. 配置数据库 provider
3. 生成数据库 snapshot
4. 基于本地图索引执行搜索、SQL 校验、影响分析和审查报告

DbGraph 默认保存的是 schema 元数据、SQL artifact、图关系和 profile 摘要。`validate-sql` 不会执行 SQL，默认也不会保存业务行数据。

完整 `.dbgraph/dbgraph.config.json` 字段说明见 [configuration.md](configuration.md)。

## 安装或直接运行

推荐方式是不依赖 Node.js，也不需要本机安装 Rust toolchain，直接下载对应系统的 release 二进制：

```bash
# macOS / Linux
curl -fsSL https://raw.githubusercontent.com/zaenzhang/dbgraph/master/install.sh | sh
```

```powershell
# Windows PowerShell
irm https://raw.githubusercontent.com/zaenzhang/dbgraph/master/install.ps1 | iex
```

如果只是临时试用或做 CI smoke check，也可以用 npx 直接从 GitHub 运行：

```bash
npx github:zaenzhang/dbgraph --version
npx github:zaenzhang/dbgraph init -i --yes
```

npm 包正式发布后，可以改用：

```bash
npx @dbgraph/cli --version
npm i -g @dbgraph/cli
```

如果你是在当前仓库里开发或测试，可以用 Cargo 前缀运行：

```powershell
cargo run -p dbgraph-cli -- --version
cargo run -p dbgraph-cli -- init -i --yes
```

如果已经把 `dbgraph` 安装到 `PATH`，可以直接运行：

```powershell
dbgraph --version
dbgraph init -i --yes
```

## 初始化项目

在你想让 DbGraph 索引的应用项目或数据库项目目录下运行：

```powershell
dbgraph init -i --yes
```

配置 Agent MCP 时不需要自己打印或复制 JSON，直接运行：

```powershell
dbgraph install --target codex --yes
dbgraph install --target cursor --yes
dbgraph install --target claude --yes
```

Agent MCP 配置里默认会写入 `command: "dbgraph"`，所以运行 `dbgraph install` 前需要先通过安装脚本或未来的包管理器安装方式让 `dbgraph` 出现在 `PATH`。

该命令会创建：

- `.dbgraph/dbgraph.config.json`
- `.dbgraph/snapshots/`
- `.dbgraph/instructions/`
- 第一次成功 snapshot 后会生成 `.dbgraph/dbgraph.db`

查看当前项目状态：

```powershell
dbgraph status
dbgraph status --json
```

## 配置数据库

DbGraph 从 `.dbgraph/dbgraph.config.json` 读取配置。

配置文件有四个顶层部分：

- `database`：provider 和连接来源。
- `snapshot`：JSON 输出、profile 深度和采样限制。
- `security`：原始数据存储、采样 mask 和自定义敏感词。
- `mcp`：MCP 开关和响应大小预算。
- `dataAccess`：用于业务行采样的表/列显式 allowlist。

### PostgreSQL

交互式默认配置会使用 `DATABASE_URL`：

```powershell
$env:DATABASE_URL="postgres://postgres:postgres@localhost:55432/teashop"
dbgraph snapshot --profile stats
```

### SQLite

SQLite 不需要外部服务。把 `.dbgraph/dbgraph.config.json` 设置为类似下面的配置：

```json
{
  "version": 1,
  "database": {
    "provider": "sqlite",
    "connectionEnv": null,
    "connectionString": "C:/path/to/app.sqlite"
  },
  "snapshot": {
    "prettyJson": true,
    "profilingMode": "schema",
    "maxRowsPerTable": 20,
    "sampleRows": false
  },
  "security": {
    "storeRawData": false,
    "storeRawSamples": false,
    "maskPii": true,
    "customSensitiveTerms": []
  },
  "mcp": {
    "enabled": true,
    "maxResponseChars": 15000
  }
}
```

MySQL 和 SQL Server 目前已经注册为 provider，但当前构建里会显式跳过，等本地或容器化测试服务补齐后再启用。

## 生成 Snapshot

```powershell
dbgraph snapshot
dbgraph snapshot --profile schema
dbgraph snapshot --profile stats
dbgraph snapshot --profile sample --max-rows-per-table 20
```

Profile 模式：

- `schema`：只采集 schema 元数据，默认且最安全。
- `stats`：采集 provider/catalog 统计信息，例如行数估计。
- `sample`：允许采样，但只有匹配 `dataAccess` allowlist 的表和列才会读取行值；默认仍会进行敏感信息 mask。

Snapshot 会写入 `.dbgraph/snapshots/`，本地图索引会重建到 `.dbgraph/dbgraph.db`。

### 显式授权的数据 Profiling

默认情况下，DbGraph 不读取业务行值。如果你希望基于少量样本做更深入的业务规则分析，需要同时开启 sample profile，并在 `dataAccess` 里指定具体表和列：

这也是给 AI agent 做数据库暴露过滤的推荐方式：敏感或暂不需要深入分析的表保持 `schemaOnly`，只提供表结构、字段、约束、索引、关系和 SQL lineage；确实允许深入分析的表才设置为 `sample`，并且只能读取配置里列出的字段。后续 `context`、MCP 工具和 `analyze` 都只复用 snapshot 中已经授权生成的摘要，不会额外绕过配置去读取行数据。

```json
{
  "snapshot": {
    "profilingMode": "sample",
    "maxRowsPerTable": 50,
    "sampleRows": true
  },
  "dataAccess": {
    "defaultMode": "schemaOnly",
    "tables": [
      {
        "pattern": "public.orders",
        "mode": "sample",
        "columns": ["status", "created_at"],
        "where": "created_at >= now() - interval '30 days'",
        "limit": 50,
        "storeRawValues": false
      },
      {
        "pattern": "public.payments",
        "mode": "schemaOnly"
      }
    ]
  }
}
```

然后运行：

```powershell
dbgraph snapshot --profile sample
dbgraph analyze --format markdown
```

当授权样本显示出 enum-like 但缺少约束、高空值率、金额类负数异常或 ID/code/email 等格式不稳定时，`analyze` 会增加 `Data Profiling & Business Rules` 报告分区。

### 业务语义 Metadata

如果字段名和表结构不足以表达业务含义，可以新增 `.dbgraph/semantics.json`：

```json
{
  "version": 1,
  "objects": [
    {
      "object": "public.orders.status",
      "description": "订单生命周期状态",
      "owner": "commerce",
      "allowedValues": ["pending", "paid", "shipped", "cancelled"],
      "deprecated": false,
      "certified": true
    }
  ]
}
```

修改后刷新 snapshot：

```powershell
dbgraph snapshot
dbgraph context "order status"
dbgraph analyze --scope quality
```

`context` 和 MCP context 响应会包含这些语义信息。如果某个对象被标记为 `deprecated: true`，但 SQL artifact 仍然引用它，`analyze` 会报告 `quality.deprecated_object_used`。

## SQL Artifact

生成 snapshot 时，DbGraph 默认扫描这些目录下的 `.sql` 文件：

- `migrations/`
- `sql/`
- `db/`

它会忽略噪声目录，例如：

- `node_modules/`
- `target/`
- `bin/`
- `obj/`

SQL artifact 会成为图里的 query object。DbGraph 会尽量提取读、写、过滤、JOIN 等依赖关系。

## 常用 CLI 命令

搜索图对象：

```powershell
dbgraph search customer
dbgraph search orders --kind table
dbgraph search email --kind column --json
```

查看表结构：

```powershell
dbgraph table public.orders
dbgraph table orders --json
```

查看关系：

```powershell
dbgraph relations public.orders --depth 2
dbgraph relations public.orders --direction incoming
```

为 AI 任务构建紧凑上下文：

```powershell
dbgraph context "refund payment order" --tokens 800
dbgraph context "which tables are touched by order status changes" --json
```

校验 SQL，但不执行 SQL：

```powershell
dbgraph validate-sql --sql "select * from orders"
dbgraph validate-sql --file sql/orders.sql --dialect postgres --json
```

比较最新 snapshot 和上一个 snapshot：

```powershell
dbgraph diff
dbgraph diff --json
```

修改对象前做影响分析：

```powershell
dbgraph impact public.orders.status
dbgraph impact public.orders.status --depth 2 --json
```

## 分析报告

运行结构化分析：

```powershell
dbgraph analyze --scope all
dbgraph analyze --scope risk
dbgraph analyze --scope quality
dbgraph analyze --scope performance
```

输出格式：

```powershell
dbgraph analyze --scope all --format text
dbgraph analyze --scope all --format json
dbgraph analyze --scope all --json
dbgraph analyze --scope all --format markdown --output dbgraph-analysis.md
```

CI gate 和已知风险 suppression：

```powershell
dbgraph analyze --fail-on high
dbgraph analyze --fail-on-new medium --baseline .dbgraph/analysis-baseline.json
dbgraph analyze --include-suppressed --suppressions .dbgraph/suppressions.json
```

分析报告包含：

- 总览和风险分数
- 分区摘要
- Top findings
- 严重程度计数
- 证据
- 影响说明
- 置信度
- 建议修复方式
- 可关联的 SQL/schema 对象

当前规则分组：

- Security & Privacy：敏感列、SQL 读取敏感列、宽泛的 `SELECT *`。
- Data Integrity & Schema Quality：缺失主键、疑似缺失外键。
- SQL Workload & Safety：没有 `WHERE` 的 `UPDATE` 或 `DELETE`。
- Performance：过滤或 JOIN 使用的列缺少支撑索引。

## 离线 Agent Benchmark

对比“直接读取原始项目文件”和“使用 DbGraph 结构化上下文”的上下文成本与证据覆盖，不调用真实 LLM：

```powershell
dbgraph benchmark-agent --scenario teashop --format markdown --output dbgraph-agent-benchmark.md
dbgraph benchmark-agent --scenario teashop --format json
```

报告包含 estimated tokens、retrieval steps、evidence recall、precision 和 token reduction。英文方法说明见 [agent-benchmark.md](agent-benchmark.md)。

## MCP Server

启动 stdio MCP server：

```powershell
dbgraph serve --mcp
```

当前 MCP 工具：

- `dbgraph_status`
- `dbgraph_search`
- `dbgraph_table`
- `dbgraph_context`
- `dbgraph_relations`
- `dbgraph_impact`
- `dbgraph_analyze`
- `dbgraph_diff`
- `dbgraph_validate_sql`

MCP 响应是 JSON text content。较大的响应会包含 `responseBudget` 元数据和建议的后续调用。

## PostgreSQL Teashop Smoke Test

运行示例数据库：

```powershell
docker compose -f examples/postgres-teashop/docker-compose.yml up -d
$env:DATABASE_URL="postgres://postgres:postgres@localhost:55432/teashop"
powershell -ExecutionPolicy Bypass -File scripts/integration/postgres-smoke.ps1
docker compose -f examples/postgres-teashop/docker-compose.yml down -v
```

Smoke test 会在临时目录初始化 DbGraph 项目，生成 PostgreSQL snapshot，运行搜索，校验 SQL，并验证结构化分析报告里包含预期的风险、性能 finding 和建议修复方式。

## 安全说明

- `dbgraph validate-sql` 不会执行 SQL。
- `dbgraph analyze` 只基于本地 snapshot 和图索引工作。
- `dbgraph snapshot` 是会连接配置数据库的命令。
- 默认关闭 raw sample 存储。
- 显式开启采样时，敏感样本默认会被 mask。
- SQLite provider 会拒绝 snapshot `.dbgraph/dbgraph.db`，避免把 DbGraph 内部图数据库误当成业务 SQLite 数据库。
