//! The vectors of the MCP gateway's tool texts, keyed by the model and the hash of the text: an
//! unchanged tool is embedded once and the vector is read back after a restart. `vector` is a JSON
//! array of numbers, as for every other body in the store.

use rusqlite::params;

use crate::{Store, StoreError, to_i64};

/// Hashes per `IN (...)` statement.
const CHUNK: usize = 500;

pub(crate) const MIGRATIONS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS mcp_tool_vectors (
        model TEXT NOT NULL,
        text_hash TEXT NOT NULL,
        dim INTEGER NOT NULL,
        upstream_model TEXT NOT NULL,
        vector TEXT NOT NULL,
        created_us INTEGER NOT NULL,
        last_used_us INTEGER NOT NULL,
        PRIMARY KEY (model, text_hash))",
    "CREATE INDEX IF NOT EXISTS mcp_tool_vectors_used ON mcp_tool_vectors (last_used_us)",
];

pub(crate) const PG_MIGRATION: &str = "CREATE TABLE IF NOT EXISTS mcp_tool_vectors (
        model text NOT NULL,
        text_hash text NOT NULL,
        dim integer NOT NULL,
        upstream_model text NOT NULL,
        vector text NOT NULL,
        created_us bigint NOT NULL,
        last_used_us bigint NOT NULL,
        PRIMARY KEY (model, text_hash));
     CREATE INDEX IF NOT EXISTS mcp_tool_vectors_used ON mcp_tool_vectors (last_used_us);";

/// The columns a load reads, before the vector is decoded.
struct Stored {
    text_hash: String,
    dim: i64,
    vector: String,
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Stored> {
    Ok(Stored {
        text_hash: r.get(0)?,
        dim: r.get(1)?,
        vector: r.get(2)?,
    })
}

fn pg_row(r: &tokio_postgres::Row) -> Result<Stored, StoreError> {
    let dim: i32 = r.try_get(1)?;
    Ok(Stored {
        text_hash: r.try_get(0)?,
        dim: i64::from(dim),
        vector: r.try_get(2)?,
    })
}

/// A row's vector, when the stored dimension still describes the body and the body parses. A row
/// that fails either is a miss: the caller embeds the text again and `put` rewrites the row.
fn decode(s: &Stored) -> Option<Vec<f32>> {
    let vector: Vec<f32> = match serde_json::from_str(&s.vector) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!(text_hash = s.text_hash, error = %e, "mcp tool vector is unreadable");
            return None;
        }
    };
    let dim = usize::try_from(s.dim).ok()?;
    if vector.len() != dim {
        tracing::debug!(
            text_hash = s.text_hash,
            dim,
            len = vector.len(),
            "mcp tool vector dimension disagrees with its body"
        );
        return None;
    }
    Some(vector)
}

impl Store {
    /// The stored vectors of `hashes` for `model` and the resolved `upstream_model`, by hash. Rows
    /// written for another upstream model, whose dimension disagrees or whose body is unreadable
    /// are not returned; the caller treats each as a miss and embeds it again.
    pub async fn load_tool_vectors(
        &self,
        model: &str,
        upstream_model: &str,
        hashes: &[String],
    ) -> Result<Vec<(String, Vec<f32>)>, StoreError> {
        if hashes.is_empty() {
            return Ok(Vec::new());
        }
        match self {
            Store::Sqlite(s) => {
                let (model, upstream) = (model.to_owned(), upstream_model.to_owned());
                let chunks: Vec<Vec<String>> = hashes.chunks(CHUNK).map(<[String]>::to_vec).collect();
                s.call(move |c| {
                    let mut out = Vec::new();
                    for chunk in &chunks {
                        let marks: Vec<String> = (3..chunk.len() + 3).map(|i| format!("?{i}")).collect();
                        let mut st = c.prepare(&format!(
                            "SELECT text_hash, dim, vector FROM mcp_tool_vectors
                             WHERE model = ?1 AND upstream_model = ?2 AND text_hash IN ({})",
                            marks.join(", ")
                        ))?;
                        let mut values: Vec<&dyn rusqlite::ToSql> = vec![&model, &upstream];
                        values.extend(chunk.iter().map(|h| h as &dyn rusqlite::ToSql));
                        let rows = st.query_map(rusqlite::params_from_iter(values), row)?;
                        for r in rows {
                            let stored = r?;
                            if let Some(vector) = decode(&stored) {
                                out.push((stored.text_hash, vector));
                            }
                        }
                    }
                    Ok(out)
                })
                .await
            }
            Store::Postgres(p) => {
                let mut out = Vec::new();
                for chunk in hashes.chunks(CHUNK) {
                    let rows = p
                        .rows(
                            "SELECT text_hash, dim, vector FROM mcp_tool_vectors
                             WHERE model = $1 AND upstream_model = $2 AND text_hash = ANY($3)",
                            &[&model, &upstream_model, &chunk.to_vec()],
                        )
                        .await?;
                    for stored in rows.iter().map(pg_row).collect::<Result<Vec<_>, _>>()? {
                        if let Some(vector) = decode(&stored) {
                            out.push((stored.text_hash, vector));
                        }
                    }
                }
                Ok(out)
            }
        }
    }

    /// Writes `(text hash, dimension, vector)` rows for `model`. An existing row is replaced, so one
    /// written with the wrong dimension heals instead of being re-embedded forever; `created_us`
    /// keeps the first write and `last_used_us` follows `now_us`.
    pub async fn put_tool_vectors(
        &self,
        model: &str,
        upstream_model: &str,
        items: &[(String, usize, Vec<f32>)],
        now_us: i64,
    ) -> Result<(), StoreError> {
        if items.is_empty() {
            return Ok(());
        }
        match self {
            Store::Sqlite(s) => {
                let (model, upstream, items) = (model.to_owned(), upstream_model.to_owned(), items.to_vec());
                s.call(move |c| {
                    let tx = c.transaction()?;
                    {
                        let mut st = tx.prepare(
                            "INSERT INTO mcp_tool_vectors
                                 (model, text_hash, dim, upstream_model, vector, created_us, last_used_us)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                             ON CONFLICT (model, text_hash) DO UPDATE SET
                                 dim = excluded.dim, upstream_model = excluded.upstream_model,
                                 vector = excluded.vector, last_used_us = excluded.last_used_us",
                        )?;
                        for (hash, dim, vector) in &items {
                            st.execute(params![
                                model,
                                hash,
                                to_i64(*dim as u64),
                                upstream,
                                serde_json::to_string(vector)?,
                                now_us,
                                now_us
                            ])?;
                        }
                    }
                    tx.commit()?;
                    Ok(())
                })
                .await
            }
            Store::Postgres(p) => {
                let mut hashes = Vec::with_capacity(items.len());
                let mut dims = Vec::with_capacity(items.len());
                let mut bodies = Vec::with_capacity(items.len());
                for (hash, dim, vector) in items {
                    hashes.push(hash.clone());
                    dims.push(i32::try_from(*dim).unwrap_or(i32::MAX));
                    bodies.push(serde_json::to_string(vector)?);
                }
                p.execute(
                    "INSERT INTO mcp_tool_vectors
                         (model, text_hash, dim, upstream_model, vector, created_us, last_used_us)
                     SELECT $1, r.h, r.d, $2, r.v, $3, $4
                       FROM unnest($5::text[], $6::int4[], $7::text[]) AS r(h, d, v)
                     ON CONFLICT (model, text_hash) DO UPDATE SET
                         dim = excluded.dim, upstream_model = excluded.upstream_model,
                         vector = excluded.vector, last_used_us = excluded.last_used_us",
                    &[&model, &upstream_model, &now_us, &now_us, &hashes, &dims, &bodies],
                )
                .await
                .map(|_| ())
            }
        }
    }

    /// Moves `last_used_us` of the stored rows of `hashes` for `model`.
    pub async fn touch_tool_vectors(
        &self,
        model: &str,
        hashes: &[String],
        now_us: i64,
    ) -> Result<(), StoreError> {
        if hashes.is_empty() {
            return Ok(());
        }
        match self {
            Store::Sqlite(s) => {
                let (model, chunks) = (
                    model.to_owned(),
                    hashes.chunks(CHUNK).map(<[String]>::to_vec).collect::<Vec<_>>(),
                );
                s.call(move |c| {
                    for chunk in &chunks {
                        let marks: Vec<String> = (3..chunk.len() + 3).map(|i| format!("?{i}")).collect();
                        let mut values: Vec<&dyn rusqlite::ToSql> = vec![&now_us, &model];
                        values.extend(chunk.iter().map(|h| h as &dyn rusqlite::ToSql));
                        c.execute(
                            &format!(
                                "UPDATE mcp_tool_vectors SET last_used_us = ?1
                                 WHERE model = ?2 AND text_hash IN ({})",
                                marks.join(", ")
                            ),
                            rusqlite::params_from_iter(values),
                        )?;
                    }
                    Ok(())
                })
                .await
            }
            Store::Postgres(p) => {
                for chunk in hashes.chunks(CHUNK) {
                    p.execute(
                        "UPDATE mcp_tool_vectors SET last_used_us = $1 WHERE model = $2 AND text_hash = ANY($3)",
                        &[&now_us, &model, &chunk.to_vec()],
                    )
                    .await?;
                }
                Ok(())
            }
        }
    }

    /// Deletes every row last used before `before_us`, returning how many went.
    pub async fn prune_tool_vectors(&self, before_us: i64) -> Result<u64, StoreError> {
        match self {
            Store::Sqlite(s) => {
                s.call(move |c| {
                    Ok(c.execute(
                        "DELETE FROM mcp_tool_vectors WHERE last_used_us < ?1",
                        params![before_us],
                    )? as u64)
                })
                .await
            }
            Store::Postgres(p) => {
                p.execute(
                    "DELETE FROM mcp_tool_vectors WHERE last_used_us < $1",
                    &[&before_us],
                )
                .await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::pg::{Migrate, PgSettings};

    fn embedded(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!(
            "antirion-store-{name}-{:x}",
            antirion_core::ResourceUid::random().0
        ));
        Store::open_embedded(&dir).unwrap()
    }

    /// A store on a fresh database of `ANTIRION_TEST_POSTGRES_URL`, when it is set.
    async fn postgres() -> Option<Store> {
        let base = std::env::var("ANTIRION_TEST_POSTGRES_URL").ok()?;
        let name = format!("t_{:x}", antirion_core::ResourceUid::random().0);
        let (client, conn) = tokio_postgres::connect(&base, tokio_postgres::NoTls).await.ok()?;
        let mut set = tokio::task::JoinSet::new();
        set.spawn(async move {
            let _ = conn.await;
        });
        client
            .batch_execute(&format!("CREATE DATABASE {name}"))
            .await
            .ok()?;
        let (prefix, _) = base.rsplit_once('/')?;
        let settings = PgSettings {
            url: format!("{prefix}/{name}"),
            pool_size: 2,
            connect_timeout: Duration::from_secs(5),
            statement_timeout: Duration::from_secs(5),
            migrate: Migrate::Auto,
            migrate_timeout: Duration::from_secs(30),
            binary_version: "0.1.1".into(),
            node_id: "n".into(),
        };
        Some(Store::open_postgres(&settings).await.unwrap())
    }

    fn hash(s: &str) -> String {
        s.to_owned()
    }

    async fn vectors_behave(store: &Store) {
        let items = vec![
            (hash("h1"), 3, vec![1.0, 0.0, 0.5]),
            (hash("h2"), 3, vec![0.0, 1.0, 0.25]),
        ];
        store.put_tool_vectors("m", "up", &items, 10).await.unwrap();
        let mut loaded = store
            .load_tool_vectors("m", "up", &[hash("h3"), hash("h1"), hash("h2")])
            .await
            .unwrap();
        loaded.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            loaded,
            vec![
                (hash("h1"), vec![1.0, 0.0, 0.5]),
                (hash("h2"), vec![0.0, 1.0, 0.25])
            ]
        );
        // Another model, alias or upstream, and an unknown hash are misses.
        assert!(
            store
                .load_tool_vectors("m", "other", &[hash("h1")])
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .load_tool_vectors("other", "up", &[hash("h1")])
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .load_tool_vectors("m", "up", &[hash("nope")])
                .await
                .unwrap()
                .is_empty()
        );
        assert!(store.load_tool_vectors("m", "up", &[]).await.unwrap().is_empty());

        // A row whose stored dimension disagrees with its body is a miss, and writing it again
        // replaces the row instead of leaving it unreadable forever.
        store
            .put_tool_vectors("m", "up", &[(hash("bad"), 8, vec![1.0, 2.0])], 11)
            .await
            .unwrap();
        assert!(
            store
                .load_tool_vectors("m", "up", &[hash("bad")])
                .await
                .unwrap()
                .is_empty()
        );
        store
            .put_tool_vectors("m", "up", &[(hash("bad"), 2, vec![1.0, 2.0])], 12)
            .await
            .unwrap();
        assert_eq!(
            store.load_tool_vectors("m", "up", &[hash("bad")]).await.unwrap(),
            vec![(hash("bad"), vec![1.0, 2.0])]
        );

        // h1 moves to 100; h2 (10) and bad (12) stay old.
        store.touch_tool_vectors("m", &[hash("h1")], 100).await.unwrap();
        assert_eq!(store.prune_tool_vectors(100).await.unwrap(), 2);
        assert_eq!(
            store
                .load_tool_vectors("m", "up", &[hash("h1"), hash("h2"), hash("bad")])
                .await
                .unwrap(),
            vec![(hash("h1"), vec![1.0, 0.0, 0.5])]
        );
        assert_eq!(store.prune_tool_vectors(101).await.unwrap(), 1);
        assert!(
            store
                .load_tool_vectors("m", "up", &[hash("h1")])
                .await
                .unwrap()
                .is_empty()
        );
    }

    /// More hashes than one `IN (...)` statement carries.
    async fn chunks_behave(store: &Store) {
        let hashes: Vec<String> = (0..CHUNK + 1).map(|i| format!("c{i}")).collect();
        let items: Vec<(String, usize, Vec<f32>)> =
            hashes.iter().map(|h| (h.clone(), 2, vec![0.5, 1.0])).collect();
        store.put_tool_vectors("m", "up", &items, 20).await.unwrap();
        assert_eq!(
            store.load_tool_vectors("m", "up", &hashes).await.unwrap().len(),
            CHUNK + 1
        );
        store.touch_tool_vectors("m", &hashes, 30).await.unwrap();
        assert_eq!(store.prune_tool_vectors(31).await.unwrap(), (CHUNK + 1) as u64);
    }

    #[tokio::test]
    async fn vectors_behave_on_the_embedded_store() {
        let store = embedded("mcp-index");
        vectors_behave(&store).await;
        chunks_behave(&store).await;
    }

    #[tokio::test]
    async fn vectors_behave_on_postgres() {
        let Some(store) = postgres().await else {
            return;
        };
        vectors_behave(&store).await;
        chunks_behave(&store).await;
    }
}
