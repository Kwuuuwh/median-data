use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

use crate::schema::{DDL_STATEMENTS, SCHEMA_VERSION};

/// One catalog item row.
pub struct CatalogRow {
    /// DE stable key (primary key).
    pub unique_name: String,
    /// Derived category value.
    pub category: String,
    /// Ducat value, if joined from a prime recipe.
    pub ducat: Option<i64>,
    /// WFM trade slug, if bridged.
    pub wfm_url_name: Option<String>,
    /// Tradable on WFM (derived from a successful bridge).
    pub tradable: bool,
    /// Raw DE texture location, if joined.
    pub icon: Option<String>,
}

/// One localized-name row.
pub struct NameRow {
    /// DE stable key this name belongs to.
    pub unique_name: String,
    /// BCP-ish language tag (`en`, `ru`, ...).
    pub lang: String,
    /// Name origin: `DE` or `WFM`.
    pub source: &'static str,
    /// Display name.
    pub name: String,
}

/// An assembled catalog ready to be written to SQLite.
pub struct CatalogData {
    /// Item rows.
    pub items: Vec<CatalogRow>,
    /// Localized-name rows.
    pub names: Vec<NameRow>,
    /// Identity of the DE index this catalog was built from.
    pub de_index_hash: String,
    /// Languages included (EN always present).
    pub langs: Vec<String>,
    /// Build time, unix epoch milliseconds.
    pub built_at_ms: i64,
}

/// Write the assembled catalog to a fresh SQLite file at `path` (replacing any existing one).
pub async fn write_catalog(path: &Path, data: &CatalogData) -> anyhow::Result<()> {
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;

    for stmt in DDL_STATEMENTS {
        sqlx::query(stmt).execute(&pool).await?;
    }

    let mut tx = pool.begin().await?;

    for item in &data.items {
        sqlx::query(
            "INSERT INTO items (unique_name, category, ducat, wfm_url_name, tradable, icon) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&item.unique_name)
        .bind(item.category.as_str())
        .bind(item.ducat)
        .bind(item.wfm_url_name.as_deref())
        .bind(item.tradable as i64)
        .bind(item.icon.as_deref())
        .execute(&mut *tx)
        .await?;
    }

    for name in &data.names {
        sqlx::query(
            "INSERT OR IGNORE INTO item_names (unique_name, lang, source, name) VALUES (?, ?, ?, ?)",
        )
        .bind(&name.unique_name)
        .bind(&name.lang)
        .bind(name.source)
        .bind(&name.name)
        .execute(&mut *tx)
        .await?;
    }

    let meta = [
        ("schema_version", SCHEMA_VERSION.to_string()),
        ("built_at", data.built_at_ms.to_string()),
        ("de_index_hash", data.de_index_hash.clone()),
        ("langs", data.langs.join(",")),
    ];
    for (key, value) in meta {
        sqlx::query("INSERT INTO meta (key, value) VALUES (?, ?)")
            .bind(key)
            .bind(value)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    pool.close().await;
    Ok(())
}
