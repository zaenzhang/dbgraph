"""Offline CLI/MCP acceptance on disposable SQLite projects; no model or business data.

Expected evidence is asserted only after retrieval. It never selects search results.
Run with --bin PATH; optionally compare payload sizes with --baseline-bin PATH.
"""

import argparse
from contextlib import closing
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin", required=True, type=Path)
    parser.add_argument("--baseline-bin", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    binary = args.bin.resolve()
    calls = []
    journeys = []

    def cli(root, *command, executable=binary, expected_code=0):
        start = time.perf_counter()
        process = subprocess.run(
            [str(executable), command[0], str(root), *command[1:],
             *([] if command[0] == "init" else ["--json"])],
            capture_output=True, encoding="utf-8", timeout=30,
        )
        body = process.stdout.strip()
        calls.append({"transport": "cli", "binary": str(executable),
                      "arguments": [command[0], str(root), *command[1:]],
                      "elapsedMs": round((time.perf_counter() - start) * 1000, 3),
                      "bytes": len(body.encode()), "estimatedTokens": (len(body.encode()) + 3) // 4,
                      "exitCode": process.returncode, "body": body, "stderr": process.stderr})
        assert process.returncode == expected_code, (command, process.stderr, body)
        if command[0] == "init" or (expected_code != 0 and not body):
            return {}
        return json.loads(body)

    def prepare(root, schema="", sql_files=None, semantics=None):
        root.mkdir()
        cli(root, "init")
        database = root / "fixture.sqlite"
        with closing(sqlite3.connect(database)) as connection:
            connection.executescript(schema)
        config_path = root / ".dbgraph/dbgraph.config.json"
        config = json.loads(config_path.read_text(encoding="utf-8"))
        config["database"].update(provider="sqlite", connectionEnv=None, connectionString=str(database))
        config["mcp"]["maxResponseChars"] = 15000
        config_path.write_text(json.dumps(config), encoding="utf-8")
        if sql_files:
            (root / "sql").mkdir()
            for name, sql in sql_files.items():
                (root / "sql" / name).write_text(sql, encoding="utf-8")
        if semantics:
            (root / ".dbgraph/semantics.json").write_text(
                json.dumps({"version": 1, "objects": semantics}), encoding="utf-8")
        cli(root, "snapshot")
        return config_path

    with tempfile.TemporaryDirectory(prefix="dbgraph-context-evidence-") as directory:
        work = Path(directory)
        project = work / "commerce"
        config_path = prepare(project, """
            CREATE TABLE customers(id INTEGER PRIMARY KEY, email TEXT);
            CREATE TABLE orders(id INTEGER PRIMARY KEY, customer_id INTEGER REFERENCES customers(id), status TEXT);
            CREATE TABLE payments(id INTEGER PRIMARY KEY, order_id INTEGER REFERENCES orders(id), amount NUMERIC);
            CREATE TABLE refunds(id INTEGER PRIMARY KEY, payment_id INTEGER REFERENCES payments(id), amount NUMERIC);
            CREATE TABLE large_ledger(id INTEGER PRIMARY KEY);
        """, {"orders.sql": "SELECT o.id, o.status FROM orders o;",
                "refunds.sql": "SELECT r.id, r.amount FROM refunds r;"},
            [{"object": "main.refunds", "description": "退款流水 refund payment records"},
             {"object": "main.large_ledger", "description": "ledger " * 4000}])
        empty = work / "empty"
        prepare(empty)
        # SQLite captures a main schema even without tables. Exercise that case
        # separately, then build an explicitly empty synthetic snapshot model.
        schema_only = cli(empty, "search", "payment")
        assert schema_only["diagnostics"]["status"] == "no_term_match"
        snapshot_path, = (empty / ".dbgraph/snapshots").glob("*.json")
        empty_model = json.loads(snapshot_path.read_text(encoding="utf-8"))
        for field in ("objects", "edges", "tableProfiles", "columnProfiles"):
            empty_model[field] = []
        empty_model.pop("schemaHash", None)
        snapshot_path.write_text(json.dumps(empty_model), encoding="utf-8")
        with closing(sqlite3.connect(empty / ".dbgraph/dbgraph.db")) as index:
            with index:
                for table in ("edges", "objects", "table_profiles", "column_profiles"):
                    index.execute(f"DELETE FROM {table} WHERE snapshot_id = ?", (empty_model["id"],))
                if index.execute("SELECT 1 FROM sqlite_master WHERE name = 'object_fts'").fetchone():
                    index.execute("DELETE FROM object_fts WHERE snapshot_id = ?", (empty_model["id"],))
                index.execute("UPDATE snapshots SET schema_hash = NULL WHERE id = ?", (empty_model["id"],))
        server = subprocess.Popen([str(binary), "serve", "--mcp"], cwd=work,
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.PIPE, encoding="utf-8", bufsize=1)

        def mcp(name, arguments, error=False):
            request = {"jsonrpc": "2.0", "id": len(calls) + 1, "method": "tools/call",
                       "params": {"name": name, "arguments": arguments}}
            start = time.perf_counter()
            server.stdin.write(json.dumps(request, ensure_ascii=False) + "\n")
            server.stdin.flush()
            line = server.stdout.readline()
            response = json.loads(line)["result"]
            body = response["content"][0]["text"]
            calls.append({"transport": "mcp", "tool": name, "arguments": arguments,
                          "elapsedMs": round((time.perf_counter() - start) * 1000, 3),
                          "bytes": len(body.encode()), "chars": len(body),
                          "estimatedTokens": (len(body.encode()) + 3) // 4,
                          "wireBytes": len(line.encode()), "body": body})
            assert response.get("isError", False) == error, response
            return json.loads(body)

        def check_suggestions(report):
            # Follow the returned calls as-is, with a server cwd outside the project.
            for suggestion in report["suggestedNextTools"]:
                result = mcp(suggestion["tool"], suggestion["arguments"])
                arguments = suggestion["arguments"]
                assert Path(arguments["projectPath"]).samefile(project)
                if suggestion["tool"] == "dbgraph_table":
                    assert result["table"] == arguments["table"]
                    assert result["columns"]
                elif suggestion["tool"] == "dbgraph_validate_sql":
                    assert result["valid"] and result["executed"] is False
                else:
                    assert result["target"]

        try:
            queries = ["kind:table 退款", "不存在的概念", "kind:table schema:absent payment",
                       "kind:column name:id 退款", "!!!", "the and", "退款 payment unknownword",
                       "kind:table schema:main", "id", "kind:query orders"]
            reports = {}
            for query in queries:
                local = cli(project, "search", query)
                remote = mcp("dbgraph_search", {"projectPath": str(project), "query": query})
                assert local == remote, query
                assert remote["snapshot"]["liveState"] == "not_checked"
                reports[query] = remote
                check_suggestions(remote)
            assert reports["kind:table 退款"]["results"][0]["fullName"] == "main.refunds"
            for query, status in [("不存在的概念", "no_term_match"),
                                  ("kind:table schema:absent payment", "no_candidates"),
                                  ("kind:column name:id 退款", "no_term_match"),
                                  ("!!!", "no_search_terms"), ("the and", "no_search_terms")]:
                assert reports[query]["diagnostics"]["status"] == status
                assert not reports[query]["results"] and not reports[query]["suggestedNextTools"]
            assert "unknownword" in reports["退款 payment unknownword"]["diagnostics"]["unmatchedTerms"]
            sql_report = reports["kind:query orders"]
            assert sql_report["results"][0]["sourcePath"] == "sql/orders.sql"
            assert any(s["tool"] == "dbgraph_validate_sql" for s in sql_report["suggestedNextTools"])
            sql = sql_report["suggestedNextTools"][0]["arguments"]["sql"]
            assert cli(project, "validate-sql", "--sql", sql, "--dialect", "generic")["valid"]

            empty_cli = cli(empty, "search", "payment")
            empty_mcp = mcp("dbgraph_search", {"projectPath": str(empty), "query": "payment"})
            assert empty_cli == empty_mcp
            assert empty_cli["diagnostics"]["status"] == "empty_snapshot"

            for query in ["退款 payment", "不存在的概念", "kind:table schema:absent payment",
                          "kind:query orders", "kind:table ledger payment"]:
                local = cli(project, "context", query, "--tokens", "800")
                remote = mcp("dbgraph_context", {"projectPath": str(project), "query": query, "tokenBudget": 800})
                assert local == remote
                assert remote["estimatedTokens"] <= 800
                assert len(calls[-1]["body"].encode()) <= 800 * 4
                check_suggestions(remote)
                if query == "退款 payment":
                    assert remote["relationPaths"], "relationships must not all lose priority to more objects"
                if "ledger" in query:
                    assert remote["coverage"]["omittedByBudget"] > 0
                    assert all(obj["fullName"] != "main.large_ledger" for obj in remote["objects"])
            empty_context = cli(empty, "context", "payment", "--tokens", "800")
            assert empty_context == mcp("dbgraph_context", {"projectPath": str(empty), "query": "payment", "tokenBudget": 800})
            assert empty_context["diagnostics"]["status"] == "empty_snapshot"

            limited = mcp("dbgraph_search", {"projectPath": str(project), "query": "kind:table", "limit": 1})
            assert limited["diagnostics"]["coverage"]["omittedByLimit"] > 0
            mcp("dbgraph_context", {"projectPath": str(project), "query": "payment", "tokenBudget": 100}, error=True)
            cli(project, "context", "payment", "--tokens", "100", expected_code=2)
            long_query = "payment " * 2000
            mcp("dbgraph_context", {"projectPath": str(project), "query": long_query, "tokenBudget": 800}, error=True)
            config = json.loads(config_path.read_text(encoding="utf-8"))
            config["mcp"]["maxResponseChars"] = 200
            config_path.write_text(json.dumps(config), encoding="utf-8")
            error = mcp("dbgraph_search", {"projectPath": str(project), "query": "退款"}, error=True)
            assert error["error"]["kind"] == "response_budget_exceeded"
            assert calls[-1]["chars"] <= 200
            config["mcp"]["maxResponseChars"] = 15000
            config_path.write_text(json.dumps(config), encoding="utf-8")
            journey_start = len(calls)
            located = mcp("dbgraph_search", {"projectPath": str(project), "query": "kind:table 退款"})
            check_suggestions(located)
            # Use the retrieved table vocabulary, not an expected-answer list.
            related_sql = mcp("dbgraph_context", {
                "projectPath": str(project),
                "query": "kind:query " + located["results"][0]["name"], "tokenBudget": 800,
            })
            check_suggestions(related_sql)
            journeys.append({"question": "Locate the refund table and validate its captured SQL",
                             "callIndexes": list(range(journey_start, len(calls)))})
            if args.baseline_bin:
                for command in [("search", "kind:table 退款"), ("search", "不存在的概念"),
                                ("context", "退款 payment", "--tokens", "800")]:
                    cli(project, *command, executable=args.baseline_bin.resolve())
                    cli(project, *command)
        finally:
            server.stdin.close()
            server.wait(timeout=10)
            assert server.returncode == 0, server.stderr.read()
            server.stdout.close()
            server.stderr.close()

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps({"calls": calls, "journeys": journeys, "temporaryProjectsRemoved": True},
                                      ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps({"calls": len(calls), "output": str(args.output), "temporaryProjectsRemoved": True}))


if __name__ == "__main__":
    main()
