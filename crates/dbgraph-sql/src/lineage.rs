//! Scope-aware references shared by validation and snapshot extraction.

use std::collections::BTreeMap;
use std::ops::ControlFlow;

use dbgraph_core::model::{DbEdgeKind, DbObjectKind, DbSnapshot};
use sqlparser::ast::{
    Assignment, AssignmentTarget, Expr, GroupByExpr, Ident, ObjectName, Query, SelectItem,
    SelectItemQualifiedWildcardKind, SetExpr, Statement, TableFactor, TableWithJoins, Visit,
    Visitor,
};

use crate::{LineageAnalysis, LineageReference, ParseDiagnostic, SqlDialect};

// A duplicate output name has no unambiguous column reference.
type Columns = BTreeMap<String, Option<Vec<String>>>;

#[derive(Clone)]
struct Source {
    name: String,
    alias: String,
    name_visible: bool,
    columns: Columns,
    physical: bool,
}

impl Source {
    fn matches_qualifier(&self, qualifier: &str) -> bool {
        qualifier == self.alias || (self.name_visible && qualifier == self.name)
    }
}

pub(super) struct Analyzer<'a> {
    snapshot: Option<&'a DbSnapshot>,
    dialect: SqlDialect,
    scopes: Vec<Vec<Source>>,
    ctes: BTreeMap<String, Columns>,
    pub(super) result: LineageAnalysis,
}

impl<'a> Analyzer<'a> {
    pub(super) fn new(dialect: SqlDialect, snapshot: Option<&'a DbSnapshot>) -> Self {
        Self {
            snapshot,
            dialect,
            scopes: Vec::new(),
            ctes: BTreeMap::new(),
            result: LineageAnalysis {
                dialect,
                diagnostics: Vec::new(),
                references: Vec::new(),
                unsafe_writes: Vec::new(),
            },
        }
    }

    fn ident(&self, ident: &Ident) -> String {
        if ident.quote_style.is_some() || self.dialect == SqlDialect::MySql {
            ident.value.clone()
        } else {
            ident.value.to_lowercase()
        }
    }

    fn name(&self, name: &ObjectName) -> String {
        name.0
            .iter()
            .filter_map(|part| part.as_ident())
            .map(|ident| self.ident(ident))
            .collect::<Vec<_>>()
            .join(".")
    }

    fn incomplete(&mut self, detail: &str) {
        let message = format!("Incomplete reference analysis: {detail}");
        if !self
            .result
            .diagnostics
            .iter()
            .any(|item| item.message == message)
        {
            self.result.diagnostics.push(ParseDiagnostic { message });
        }
    }

    fn push(&mut self, name: String, kind: DbEdgeKind, resolved: bool) {
        let reference = LineageReference {
            object_name: name,
            kind,
            alias: None,
            resolved,
        };
        if !self.result.references.contains(&reference) {
            self.result.references.push(reference);
        }
    }

    pub(super) fn statement(&mut self, statement: &Statement) {
        self.scopes.push(Vec::new());
        match statement {
            Statement::Query(query) => {
                self.query(query);
            }
            Statement::Update {
                table,
                assignments,
                from,
                selection,
                returning,
                ..
            } => {
                self.table(table, DbEdgeKind::WritesTo);
                let targets = self.scopes.last().expect("statement scope").clone();
                if let Some(from) = from {
                    for table in crate::update_from_tables(from) {
                        self.table(table, DbEdgeKind::ReadsFrom);
                    }
                }
                self.assignments(assignments, &targets);
                if let Some(expr) = selection {
                    self.expr(expr, DbEdgeKind::FiltersBy);
                } else {
                    self.result.unsafe_writes.push("update".to_owned());
                }
                if let Some(items) = returning {
                    self.projection(items);
                }
            }
            Statement::Delete(delete) => {
                for table in crate::from_tables(&delete.from) {
                    self.table(table, DbEdgeKind::WritesTo);
                }
                if let Some(using) = &delete.using {
                    for table in using {
                        self.table(table, DbEdgeKind::ReadsFrom);
                    }
                }
                if let Some(expr) = &delete.selection {
                    self.expr(expr, DbEdgeKind::FiltersBy);
                } else {
                    self.result.unsafe_writes.push("delete".to_owned());
                }
                if let Some(items) = &delete.returning {
                    self.projection(items);
                }
            }
            Statement::Insert(insert) => {
                // TableObject includes function targets, which are not schema tables.
                if let sqlparser::ast::TableObject::TableName(name) = &insert.table {
                    self.physical(
                        &self.name(name),
                        insert.table_alias.as_ref().map(|x| self.ident(x)),
                        DbEdgeKind::WritesTo,
                    );
                } else {
                    self.incomplete("INSERT function target");
                }
                for column in &insert.columns {
                    self.column(&[self.ident(column)], DbEdgeKind::WritesTo);
                }
                let targets = self.scopes.last().expect("statement scope").clone();
                self.assignments(&insert.assignments, &targets);
                if let Some(query) = &insert.source {
                    let saved = std::mem::take(&mut self.scopes);
                    self.query(query);
                    self.scopes = saved;
                }
                if let Some(items) = &insert.returning {
                    self.projection(items);
                }
                if insert.on.is_some() {
                    self.incomplete("INSERT conflict clause");
                }
            }
            _ => self.incomplete("unsupported statement kind"),
        }
        self.scopes.pop();
    }

    fn assignments(&mut self, assignments: &[Assignment], target_sources: &[Source]) {
        for assignment in assignments {
            let targets = match &assignment.target {
                AssignmentTarget::ColumnName(name) => vec![name],
                AssignmentTarget::Tuple(names) => names.iter().collect(),
            };
            // SET targets belong to the write target, not to UPDATE ... FROM sources.
            let read_scopes = std::mem::replace(&mut self.scopes, vec![target_sources.to_vec()]);
            for name in targets {
                let parts = name
                    .0
                    .iter()
                    .filter_map(|x| x.as_ident())
                    .map(|x| self.ident(x))
                    .collect::<Vec<_>>();
                self.column(&parts, DbEdgeKind::WritesTo);
            }
            self.scopes = read_scopes;
            self.expr(&assignment.value, DbEdgeKind::ReadsFrom);
        }
    }

    fn query(&mut self, query: &Query) -> Columns {
        let previous_ctes = self.ctes.clone();
        if let Some(with) = &query.with {
            if with.recursive {
                self.incomplete("recursive CTE");
            }
            for cte in &with.cte_tables {
                let mut columns = self.query(&cte.query);
                if !cte.alias.columns.is_empty() {
                    // Do not guess positional mapping from an unordered name map.
                    self.incomplete("CTE column alias list");
                    columns.clear();
                }
                let name = self.ident(&cte.alias.name);
                if self.snapshot.is_none() {
                    self.push(name.clone(), DbEdgeKind::DependsOn, true);
                }
                self.ctes.insert(name, columns);
            }
        }
        self.scopes.push(Vec::new());
        let columns = self.set_expr(&query.body);
        if let Some(order) = &query.order_by {
            if let sqlparser::ast::OrderByKind::Expressions(items) = &order.kind {
                for item in items {
                    if let Expr::Identifier(ident) = &item.expr {
                        if let Some(origins) = columns.get(&self.ident(ident)) {
                            if let Some(origins) = origins {
                                for origin in origins {
                                    self.push(origin.clone(), DbEdgeKind::OrdersBy, true);
                                }
                            } else {
                                self.push(self.ident(ident), DbEdgeKind::OrdersBy, false);
                            }
                            continue;
                        }
                    }
                    self.expr(&item.expr, DbEdgeKind::OrdersBy);
                }
            }
        }
        self.expressions(&query.limit_clause, DbEdgeKind::ReadsFrom);
        self.expressions(&query.fetch, DbEdgeKind::ReadsFrom);
        if !query.pipe_operators.is_empty() || query.for_clause.is_some() {
            self.incomplete("query output clause");
        }
        self.scopes.pop();
        self.ctes = previous_ctes;
        columns
    }

    fn set_expr(&mut self, expr: &SetExpr) -> Columns {
        match expr {
            SetExpr::Select(select) => {
                for table in &select.from {
                    self.table(table, DbEdgeKind::ReadsFrom);
                }
                let columns = self.projection(&select.projection);
                self.expressions(&select.distinct, DbEdgeKind::ReadsFrom);
                self.expressions(&select.named_window, DbEdgeKind::ReadsFrom);
                self.expressions(&select.top, DbEdgeKind::ReadsFrom);
                self.expressions(&select.prewhere, DbEdgeKind::FiltersBy);
                self.expressions(&select.cluster_by, DbEdgeKind::GroupsBy);
                self.expressions(&select.distribute_by, DbEdgeKind::GroupsBy);
                self.expressions(&select.sort_by, DbEdgeKind::OrdersBy);
                if select.into.is_some()
                    || !select.lateral_views.is_empty()
                    || select.connect_by.is_some()
                    || select.exclude.is_some()
                {
                    self.incomplete("SELECT output or source extension");
                }
                if let Some(expr) = &select.selection {
                    self.expr(expr, DbEdgeKind::FiltersBy);
                }
                if let GroupByExpr::Expressions(items, _) = &select.group_by {
                    for item in items {
                        self.expr(item, DbEdgeKind::GroupsBy);
                    }
                }
                if let Some(expr) = &select.having {
                    self.expr(expr, DbEdgeKind::FiltersBy);
                }
                if let Some(expr) = &select.qualify {
                    self.expr(expr, DbEdgeKind::FiltersBy);
                }
                columns
            }
            SetExpr::Query(query) => self.query(query),
            SetExpr::SetOperation { left, right, .. } => {
                let left_columns = self.set_expr(left);
                self.scopes.last_mut().expect("query scope").clear();
                self.set_expr(right);
                self.incomplete("set-operation output lineage");
                left_columns
            }
            SetExpr::Values(values) => {
                for row in &values.rows {
                    for expr in row {
                        self.expr(expr, DbEdgeKind::ReadsFrom);
                    }
                }
                Columns::new()
            }
            SetExpr::Insert(statement)
            | SetExpr::Update(statement)
            | SetExpr::Delete(statement) => {
                self.statement(statement);
                Columns::new()
            }
            _ => {
                self.incomplete("unsupported query body");
                Columns::new()
            }
        }
    }

    fn projection(&mut self, items: &[SelectItem]) -> Columns {
        let mut output = Columns::new();
        for item in items {
            match item {
                SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. } => {
                    // Collect projection origins even if another clause already references them.
                    let previous = std::mem::take(&mut self.result.references);
                    self.expr(expr, DbEdgeKind::ReadsFrom);
                    let origins = self
                        .result
                        .references
                        .iter()
                        .filter(|r| r.resolved)
                        .map(|r| r.object_name.clone())
                        .collect::<Vec<_>>();
                    let references = std::mem::replace(&mut self.result.references, previous);
                    for reference in references {
                        self.push(reference.object_name, reference.kind, reference.resolved);
                    }
                    let name = match item {
                        SelectItem::ExprWithAlias { alias, .. } => Some(self.ident(alias)),
                        _ => match expr {
                            Expr::Identifier(ident) => Some(self.ident(ident)),
                            Expr::CompoundIdentifier(parts) => {
                                parts.last().map(|ident| self.ident(ident))
                            }
                            _ => None,
                        },
                    };
                    if let Some(name) = name {
                        output
                            .entry(name)
                            .and_modify(|value| *value = None)
                            .or_insert(Some(origins));
                    }
                }
                SelectItem::Wildcard(options) | SelectItem::QualifiedWildcard(_, options) => {
                    if options.opt_ilike.is_some()
                        || options.opt_exclude.is_some()
                        || options.opt_except.is_some()
                        || options.opt_replace.is_some()
                        || options.opt_rename.is_some()
                    {
                        self.incomplete("wildcard modifiers");
                    }
                    let qualifier = match item {
                        SelectItem::QualifiedWildcard(
                            SelectItemQualifiedWildcardKind::ObjectName(name),
                            _,
                        ) => Some(self.name(name)),
                        SelectItem::QualifiedWildcard(_, _) => {
                            self.incomplete("expression wildcard");
                            continue;
                        }
                        _ => None,
                    };
                    let sources = self.scopes.last().cloned().unwrap_or_default();
                    let mut found = false;
                    for source in sources {
                        if qualifier
                            .as_ref()
                            .is_some_and(|q| !source.matches_qualifier(q))
                        {
                            continue;
                        }
                        found = true;
                        for (name, origins) in source.columns {
                            if let Some(origins) = &origins {
                                for origin in origins {
                                    self.push(origin.clone(), DbEdgeKind::ReadsFrom, true);
                                }
                            }
                            output
                                .entry(name)
                                .and_modify(|value| *value = None)
                                .or_insert(origins);
                        }
                    }
                    if !found {
                        self.incomplete("wildcard has no matching source");
                    }
                }
            }
        }
        output
    }

    fn table(&mut self, table: &TableWithJoins, kind: DbEdgeKind) {
        self.factor(&table.relation, kind);
        for join in &table.joins {
            self.factor(&join.relation, DbEdgeKind::JoinsOn);
            match crate::join_constraint(&join.join_operator) {
                Some(sqlparser::ast::JoinConstraint::On(expr)) => {
                    self.expr(expr, DbEdgeKind::JoinsOn);
                }
                Some(
                    sqlparser::ast::JoinConstraint::Using(_)
                    | sqlparser::ast::JoinConstraint::Natural,
                ) => self.incomplete("JOIN column coalescing"),
                _ => (),
            }
        }
    }

    fn factor(&mut self, factor: &TableFactor, kind: DbEdgeKind) {
        match factor {
            TableFactor::Table {
                name, alias, args, ..
            } => {
                if args.is_some() {
                    self.incomplete("table function");
                }
                let name = self.name(name);
                if alias
                    .as_ref()
                    .is_some_and(|alias| !alias.columns.is_empty())
                {
                    self.incomplete("table column alias list");
                }
                let alias = alias.as_ref().map(|a| self.ident(&a.name));
                if let Some(columns) = self.ctes.get(&name).cloned() {
                    self.scopes.last_mut().expect("scope").push(Source {
                        name_visible: alias.is_none(),
                        alias: alias.unwrap_or_else(|| name.clone()),
                        name,
                        columns,
                        physical: false,
                    });
                } else {
                    self.physical(&name, alias, kind);
                }
            }
            TableFactor::Derived {
                subquery,
                alias,
                lateral,
                ..
            } => {
                let saved = if *lateral {
                    None
                } else {
                    Some(std::mem::take(&mut self.scopes))
                };
                let columns = self.query(subquery);
                if let Some(saved) = saved {
                    self.scopes = saved;
                }
                if let Some(alias) = alias {
                    if !alias.columns.is_empty() {
                        self.incomplete("derived column alias list");
                    }
                    let name = self.ident(&alias.name);
                    self.scopes.last_mut().expect("scope").push(Source {
                        alias: name.clone(),
                        name,
                        name_visible: false,
                        columns,
                        physical: false,
                    });
                } else {
                    self.incomplete("derived table without alias");
                }
            }
            TableFactor::NestedJoin {
                table_with_joins, ..
            } => self.table(table_with_joins, kind),
            _ => self.incomplete("unsupported table source"),
        }
    }

    fn physical(&mut self, name: &str, alias: Option<String>, kind: DbEdgeKind) {
        let mut canonical = name.to_owned();
        let mut columns = Columns::new();
        let mut resolved = self.snapshot.is_none();
        if let Some(snapshot) = self.snapshot {
            let tables = snapshot
                .objects
                .iter()
                .filter(|o| {
                    matches!(
                        o.kind,
                        DbObjectKind::Table | DbObjectKind::View | DbObjectKind::MaterializedView
                    )
                })
                .filter(|o| o.full_name == name || (!name.contains('.') && o.name == name))
                .collect::<Vec<_>>();
            if let [table] = tables.as_slice() {
                canonical.clone_from(&table.full_name);
                resolved = true;
                for column in snapshot
                    .objects
                    .iter()
                    .filter(|o| o.kind == DbObjectKind::Column)
                {
                    if column
                        .full_name
                        .strip_prefix(&format!("{canonical}."))
                        .is_some_and(|s| !s.contains('.'))
                    {
                        columns.insert(column.name.clone(), Some(vec![column.full_name.clone()]));
                    }
                }
            }
        }
        self.push(canonical.clone(), kind, resolved);
        let name_visible = alias.is_none();
        let alias = alias.unwrap_or_else(|| name.rsplit('.').next().unwrap_or(name).to_owned());
        self.scopes.last_mut().expect("scope").push(Source {
            name: canonical,
            alias,
            name_visible,
            columns,
            physical: true,
        });
    }

    fn column(&mut self, parts: &[String], kind: DbEdgeKind) {
        let Some(column) = parts.last() else {
            return;
        };
        let qualifier = (parts.len() > 1).then(|| parts[..parts.len() - 1].join("."));
        for scope in self.scopes.iter().rev() {
            let eligible = scope
                .iter()
                .filter(|s| qualifier.as_ref().map_or(true, |q| s.matches_qualifier(q)))
                .collect::<Vec<_>>();
            if self.snapshot.is_none() {
                if let (Some(_), [source]) = (&qualifier, eligible.as_slice()) {
                    if source.physical {
                        self.push(format!("{}.{column}", source.name), kind, true);
                        return;
                    }
                }
            }
            let matches = eligible
                .iter()
                .filter_map(|s| s.columns.get(column))
                .collect::<Vec<_>>();
            if let [Some(origins)] = matches.as_slice() {
                let origins = origins.clone();
                for origin in origins {
                    self.push(origin, kind, true);
                }
                return;
            }
            if matches.len() == 1 {
                break;
            }
            if qualifier.is_some() && matches.is_empty() {
                if let [source] = eligible.as_slice() {
                    let name = format!("{}.{column}", source.name);
                    self.push(name, kind, false);
                    return;
                }
            }
            if matches.len() > 1 || (qualifier.is_some() && !eligible.is_empty()) {
                break;
            }
        }
        self.push(parts.join("."), kind, false);
    }

    fn expr(&mut self, expr: &Expr, kind: DbEdgeKind) {
        self.expressions(expr, kind);
    }

    fn expressions(&mut self, value: &impl Visit, kind: DbEdgeKind) {
        let _ = value.visit(&mut Expressions {
            analyzer: self,
            kind,
            query_depth: 0,
        });
    }
}

struct Expressions<'a, 'b> {
    analyzer: &'a mut Analyzer<'b>,
    kind: DbEdgeKind,
    query_depth: usize,
}

impl Visitor for Expressions<'_, '_> {
    type Break = ();
    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<()> {
        if self.query_depth == 0 {
            self.analyzer.query(query);
        }
        self.query_depth += 1;
        ControlFlow::Continue(())
    }
    fn post_visit_query(&mut self, _: &Query) -> ControlFlow<()> {
        self.query_depth -= 1;
        ControlFlow::Continue(())
    }
    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
        if self.query_depth == 0 {
            let parts = match expr {
                Expr::Identifier(ident) => vec![self.analyzer.ident(ident)],
                Expr::CompoundIdentifier(parts) => {
                    parts.iter().map(|i| self.analyzer.ident(i)).collect()
                }
                _ => Vec::new(),
            };
            if !parts.is_empty() {
                self.analyzer.column(&parts, self.kind);
            }
        }
        ControlFlow::Continue(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{analyze_sql, analyze_sql_with_snapshot};
    use dbgraph_core::model::DbObject;

    fn schema() -> DbSnapshot {
        let mut snapshot = DbSnapshot::new("s1", "postgres", "app", 1);
        for (table, columns) in [
            ("orders", vec!["id", "customer_id", "status"]),
            ("customers", vec!["id", "email"]),
        ] {
            let name = format!("public.{table}");
            snapshot.objects.push(DbObject::new(
                format!("table:{table}"),
                DbObjectKind::Table,
                &name,
            ));
            for column in columns {
                let name = format!("public.{table}.{column}");
                snapshot.objects.push(DbObject::new(
                    format!("column:{name}"),
                    DbObjectKind::Column,
                    name,
                ));
            }
        }
        snapshot
    }

    #[test]
    fn rejects_unknown_and_ambiguous_references_in_actual_scope() {
        let snapshot = schema();
        for sql in [
            "select nonexistent from orders",
            "update orders set nonexistent=1",
            "select * from orders where exists (select 1 from missing_table)",
            "select * from orders o where o.email=1",
            "select id from orders o join customers c on o.customer_id=c.id",
            "select * from orders o where exists (select 1 from customers o where o.status='x')",
            "insert into orders(id) select id",
            "select orders.id from orders o",
            "select public.orders.id from orders o",
            "update orders set email='x' from customers where orders.customer_id=customers.id",
            "select c.id from (select o.id, c.id from orders o join customers c on o.customer_id=c.id) c",
            "select id, status as id from orders order by id",
            "select \"Email\" from customers",
            "select o.id from orders \"O\"",
        ] {
            let result = analyze_sql_with_snapshot(sql, SqlDialect::Postgres, &snapshot).unwrap();
            assert!(result.references.iter().any(|r| !r.resolved), "{sql}: {result:?}");
        }
    }

    #[test]
    fn resolves_projection_aliases_ctes_and_correlated_subqueries() {
        let snapshot = schema();
        for sql in [
            "select email from customers",
            "select o.id from orders o join customers c on o.customer_id=c.id",
            "with c as (select email from customers) select email from c",
            "select c.email from (select email from customers) c",
            "select o.id from orders o where exists (select 1 from customers c where c.id=o.customer_id)",
            "select email as contact from customers order by contact",
            "update orders set status=(select email from customers where id=1) where id=1",
            "insert into orders(id) values (1)",
            "update orders set id=c.id from customers c where orders.customer_id=c.id",
            "select public.orders.id from public.orders",
            "select \"O\".\"id\" from \"orders\" \"O\"",
            "select c.constant from (select 1 as constant) c",
        ] {
            let result = analyze_sql_with_snapshot(sql, SqlDialect::Postgres, &snapshot).unwrap();
            assert!(result.diagnostics.is_empty(), "{sql}: {result:?}");
            assert!(result.references.iter().all(|r| r.resolved), "{sql}: {result:?}");
        }
        let result = analyze_sql_with_snapshot(
            "select email from customers",
            SqlDialect::Postgres,
            &snapshot,
        )
        .unwrap();
        assert!(result
            .references
            .iter()
            .any(|r| r.object_name == "public.customers.email" && r.kind == DbEdgeKind::ReadsFrom));
    }

    #[test]
    fn missing_outer_where_is_not_hidden_by_nested_where_or_literals() {
        for sql in [
            "update orders set status=(select email from customers where id=1)",
            "with c as (select id from customers where id=1) delete from orders",
            "update orders set status=' WHERE '",
            "delete from orders where id=1; update orders set status='x'",
        ] {
            assert!(
                !analyze_sql(sql, SqlDialect::Postgres)
                    .unwrap()
                    .unsafe_writes
                    .is_empty(),
                "{sql}"
            );
        }
        assert!(
            analyze_sql("delete from orders where id=1", SqlDialect::Postgres)
                .unwrap()
                .unsafe_writes
                .is_empty()
        );
    }

    #[test]
    fn empty_malformed_and_unsupported_sql_report_diagnostics() {
        for sql in [
            "-- only a comment",
            "select from",
            "create table x(id int)",
            "select * from orders natural join customers",
            "select * from orders o(renamed_id)",
        ] {
            assert!(
                !analyze_sql(sql, SqlDialect::Postgres)
                    .unwrap()
                    .diagnostics
                    .is_empty(),
                "{sql}"
            );
        }
    }
}
