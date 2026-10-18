//! Rollups: one row per period, node and combination of dimension values, holding counters and
//! encoded quantile sketches. A write adds a row's counters to the stored row's and merges the
//! sketches with the caller's merge function; only the node that owns a row writes it.

use rusqlite::{Connection, OptionalExtension};

use super::ScopeRef;
use super::sql::{Arg, Sql, scope_clause};
use crate::{Store, StoreError};

/// The dimension columns, in the order of [`RollupRow::dims`].
pub const ROLLUP_DIMS: [&str; 18] = [
    "organization",
    "team",
    "project",
    "key_name",
    "user_name",
    "model",
    "requested_model",
    "provider",
    "deployment",
    "route",
    "route_node",
    "status_code",
    "code",
    "response_path",
    "cache_status",
    "environment",
    "stream",
    "cache_prefix",
];

/// Where the store keeps the time from which rollups carry `cache_prefix`: rows of earlier periods
/// were written without it.
pub const CACHE_PREFIX_FROM: &str = "rollups/cache_prefix_from";

/// The counter columns, in the order of [`RollupRow::counts`].
pub const MEASURES: [&str; 16] = [
    "requests",
    "errors",
    "cancelled",
    "fallbacks",
    "retries",
    "cache_hits",
    "guardrail_blocks",
    "guardrail_redactions",
    "guardrail_flags",
    "input",
    "input_cache_read",
    "input_cache_write",
    "output",
    "output_reasoning",
    "cost_nusd",
    "cache_saved_nusd",
];

/// The sketch columns, in the order of [`RollupRow::sketches`].
pub const SKETCHES: [&str; 3] = ["latency", "ttft", "otps"];

/// One period of one combination of dimension values on one node.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RollupRow {
    /// The period's length in seconds: 60, 3600 or 86400.
    pub grain_s: i64,
    pub start_us: i64,
    pub node_id: String,
    pub dims: Vec<Option<String>>,
    pub counts: Vec<i64>,
    pub sketches: Vec<Vec<u8>>,
}

impl RollupRow {
    /// The dimension values as one key: absent values differ from every text.
    #[must_use]
    pub fn dims_key(&self) -> String {
        let mut out = String::new();
        for d in &self.dims {
            match d {
                Some(v) => out.push_str(v),
                None => out.push('\u{1e}'),
            }
            out.push('\u{1f}');
        }
        out
    }
}

/// The rows of one grain within `[from_us, to_us)` whose dimensions match.
#[derive(Clone, Debug, Default)]
pub struct RollupQuery {
    pub grain_s: i64,
    pub from_us: i64,
    pub to_us: i64,
    /// `(index into ROLLUP_DIMS, accepted values)`.
    pub any_of: Vec<(usize, Vec<String>)>,
    pub status_classes: Vec<u8>,
    pub scopes: Option<Vec<ScopeRef>>,
    /// Rows after this one, in the order of the rows' key (start, node, dimensions).
    pub after: Option<RollupPos>,
    /// At most this many rows, in key order; `None` for every row, unordered.
    pub limit: Option<usize>,
}

/// A rollup row's key within its grain: where the next page of a query starts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RollupPos {
    pub start_us: i64,
    pub node_id: String,
    pub dims_key: String,
}

/// One page of rollup rows, with where the next page starts (the stored key of its last row,
/// which a row written by an older version may have computed over fewer dimensions).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RollupPage {
    pub rows: Vec<RollupRow>,
    pub next: Option<RollupPos>,
}

fn pos(start_us: i64, node_id: &str, dims_key: String) -> RollupPos {
    RollupPos {
        start_us,
        node_id: node_id.to_owned(),
        dims_key,
    }
}

/// Merges two encoded sketches.
pub type SketchMerge = fn(&[u8], &[u8]) -> Vec<u8>;

pub(crate) fn columns() -> String {
    let mut cols = vec!["grain_s", "start_us", "node_id", "dims_key"];
    cols.extend(ROLLUP_DIMS);
    cols.extend(MEASURES);
    cols.extend(SKETCHES);
    cols.join(", ")
}

pub(crate) fn column_count() -> usize {
    4 + ROLLUP_DIMS.len() + MEASURES.len() + SKETCHES.len()
}

pub(crate) fn query(pg: bool, q: &RollupQuery) -> Sql {
    let mut s = Sql::new(pg, &format!("SELECT {} FROM log_rollups", columns()));
    let grain = s.bind(Arg::Int(q.grain_s));
    let from = s.bind(Arg::Int(q.from_us));
    let to = s.bind(Arg::Int(q.to_us));
    s.push(&format!(
        " WHERE grain_s = {grain} AND start_us >= {from} AND start_us < {to}"
    ));
    for (i, values) in &q.any_of {
        if let Some(column) = ROLLUP_DIMS.get(*i) {
            let values: Vec<Arg> = values.iter().cloned().map(Arg::Text).collect();
            s.any_of(column, &values);
        }
    }
    if !q.status_classes.is_empty() {
        let values: Vec<Arg> = q
            .status_classes
            .iter()
            .map(|c| Arg::Text(c.to_string()))
            .collect();
        s.any_of("substr(status_code, 1, 1)", &values);
    }
    if let Some(scopes) = &q.scopes {
        scope_clause(&mut s, scopes, "organization", "team", "project");
    }
    if let Some(a) = &q.after {
        let start = s.bind(Arg::Int(a.start_us));
        let node = s.bind(Arg::Text(a.node_id.clone()));
        let key = s.bind(Arg::Text(a.dims_key.clone()));
        s.push(&format!(
            " AND (start_us, node_id, dims_key) > ({start}, {node}, {key})"
        ));
    }
    if let Some(n) = q.limit {
        let n = s.bind(Arg::Int(i64::try_from(n).unwrap_or(i64::MAX)));
        s.push(&format!(" ORDER BY start_us, node_id, dims_key LIMIT {n}"));
    }
    s
}

pub(crate) fn add_into(stored: &mut RollupRow, new: &RollupRow, merge: SketchMerge) {
    for (a, b) in stored.counts.iter_mut().zip(&new.counts) {
        *a = a.saturating_add(*b);
    }
    for (a, b) in stored.sketches.iter_mut().zip(&new.sketches) {
        *a = merge(a, b);
    }
}

pub(crate) fn args(r: &RollupRow) -> Vec<Option<Arg>> {
    let mut out = vec![
        Some(Arg::Int(r.grain_s)),
        Some(Arg::Int(r.start_us)),
        Some(Arg::Text(r.node_id.clone())),
        Some(Arg::Text(r.dims_key())),
    ];
    out.extend((0..ROLLUP_DIMS.len()).map(|i| r.dims.get(i).cloned().flatten().map(Arg::Text)));
    out.extend((0..MEASURES.len()).map(|i| Some(Arg::Int(r.counts.get(i).copied().unwrap_or(0)))));
    out
}

fn sqlite_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<RollupRow> {
    let base = 4;
    let dims = (0..ROLLUP_DIMS.len())
        .map(|i| r.get::<_, Option<String>>(base + i))
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let m = base + ROLLUP_DIMS.len();
    let counts = (0..MEASURES.len())
        .map(|i| r.get::<_, i64>(m + i))
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let s = m + MEASURES.len();
    let sketches = (0..SKETCHES.len())
        .map(|i| r.get::<_, Vec<u8>>(s + i))
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(RollupRow {
        grain_s: r.get(0)?,
        start_us: r.get(1)?,
        node_id: r.get(2)?,
        dims,
        counts,
        sketches,
    })
}

/// A row read by a query, with the next page's position after it.
pub(crate) fn pg_page(rows: &[tokio_postgres::Row]) -> Result<RollupPage, StoreError> {
    let mut page = RollupPage::default();
    for r in rows {
        let row = pg_row(r)?;
        page.next = Some(pos(row.start_us, &row.node_id, r.try_get::<_, String>(3)?));
        page.rows.push(row);
    }
    Ok(page)
}

pub(crate) fn pg_row(r: &tokio_postgres::Row) -> Result<RollupRow, StoreError> {
    let base = 4;
    let dims = (0..ROLLUP_DIMS.len())
        .map(|i| r.try_get::<_, Option<String>>(base + i))
        .collect::<Result<Vec<_>, _>>()?;
    let m = base + ROLLUP_DIMS.len();
    let counts = (0..MEASURES.len())
        .map(|i| r.try_get::<_, i64>(m + i))
        .collect::<Result<Vec<_>, _>>()?;
    let s = m + MEASURES.len();
    let sketches = (0..SKETCHES.len())
        .map(|i| r.try_get::<_, Vec<u8>>(s + i))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RollupRow {
        grain_s: r.try_get(0)?,
        start_us: r.try_get(1)?,
        node_id: r.try_get(2)?,
        dims,
        counts,
        sketches,
    })
}

/// `INSERT … ON CONFLICT DO UPDATE` replacing every value of the row.
pub(crate) fn upsert(pg: bool) -> String {
    let n = column_count();
    let marks: Vec<String> = (1..=n)
        .map(|i| if pg { format!("${i}") } else { format!("?{i}") })
        .collect();
    let mut updates: Vec<String> = MEASURES.iter().map(|m| format!("{m} = excluded.{m}")).collect();
    updates.extend(SKETCHES.iter().map(|m| format!("{m} = excluded.{m}")));
    format!(
        "INSERT INTO log_rollups ({}) VALUES ({}) ON CONFLICT (grain_s, start_us, node_id, dims_key) DO UPDATE SET {}",
        columns(),
        marks.join(", "),
        updates.join(", ")
    )
}

const SELECT_ONE: &str = "WHERE grain_s = ? AND start_us = ? AND node_id = ? AND dims_key = ?";

fn sqlite_merge(c: &mut Connection, rows: &[RollupRow], merge: SketchMerge) -> Result<(), StoreError> {
    let tx = c.transaction()?;
    {
        let mut find = tx.prepare(&format!("SELECT {} FROM log_rollups {SELECT_ONE}", columns()))?;
        let mut write = tx.prepare(&upsert(false))?;
        for new in rows {
            let stored = find
                .query_row(
                    rusqlite::params![new.grain_s, new.start_us, new.node_id, new.dims_key()],
                    sqlite_row,
                )
                .optional()?;
            let row = match stored {
                Some(mut s) => {
                    add_into(&mut s, new, merge);
                    s
                }
                None => new.clone(),
            };
            let mut values: Vec<rusqlite::types::Value> = args(&row)
                .into_iter()
                .map(|a| a.map_or(rusqlite::types::Value::Null, |a| a.sqlite()))
                .collect();
            values.extend(
                (0..SKETCHES.len())
                    .map(|i| rusqlite::types::Value::Blob(row.sketches.get(i).cloned().unwrap_or_default())),
            );
            write.execute(rusqlite::params_from_iter(values))?;
        }
    }
    tx.commit()?;
    Ok(())
}

fn sqlite_query(c: &mut Connection, q: &RollupQuery) -> Result<RollupPage, StoreError> {
    let s = query(false, q);
    let mut stmt = c.prepare(&s.text)?;
    let args: Vec<rusqlite::types::Value> = s.args.iter().map(Arg::sqlite).collect();
    let rows = stmt.query_map(rusqlite::params_from_iter(args), |r| {
        Ok((sqlite_row(r)?, r.get::<_, String>(3)?))
    })?;
    let mut page = RollupPage::default();
    let mut last = None;
    for row in rows {
        let (row, key) = row?;
        last = Some(pos(row.start_us, &row.node_id, key));
        page.rows.push(row);
    }
    page.next = last;
    Ok(page)
}

pub(crate) fn sqlite_schema() -> String {
    let dims: Vec<String> = ROLLUP_DIMS.iter().map(|d| format!("{d} TEXT")).collect();
    let measures: Vec<String> = MEASURES.iter().map(|m| format!("{m} INTEGER NOT NULL")).collect();
    let sketches: Vec<String> = SKETCHES.iter().map(|s| format!("{s} BLOB NOT NULL")).collect();
    format!(
        "CREATE TABLE IF NOT EXISTS log_rollups (
            grain_s INTEGER NOT NULL, start_us INTEGER NOT NULL, node_id TEXT NOT NULL, dims_key TEXT NOT NULL,
            {}, {}, {},
            PRIMARY KEY (grain_s, start_us, node_id, dims_key));
         CREATE INDEX IF NOT EXISTS log_rollups_start ON log_rollups (grain_s, start_us);",
        dims.join(", "),
        measures.join(", "),
        sketches.join(", ")
    )
}

impl Store {
    /// Adds each row to the stored row of the same period, node and dimensions.
    pub async fn merge_rollups(&self, rows: Vec<RollupRow>, merge: SketchMerge) -> Result<(), StoreError> {
        if rows.is_empty() {
            return Ok(());
        }
        match self {
            Store::Sqlite(s) => s.logs().call(move |c| sqlite_merge(c, &rows, merge)).await,
            Store::Postgres(p) => p.merge_rollup_rows(rows, merge).await,
        }
    }

    /// The rows `q` selects; with a limit, one page of them in key order.
    pub async fn rollups(&self, q: RollupQuery) -> Result<Vec<RollupRow>, StoreError> {
        Ok(self.rollup_page(q).await?.rows)
    }

    /// The rows `q` selects and where the next page starts: with a limit, the next page is the
    /// same query `after` [`RollupPage::next`].
    pub async fn rollup_page(&self, q: RollupQuery) -> Result<RollupPage, StoreError> {
        match self {
            Store::Sqlite(s) => s.logs().call(move |c| sqlite_query(c, &q)).await,
            Store::Postgres(p) => p.rollup_rows(&q).await,
        }
    }
}
