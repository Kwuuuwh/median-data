/// Catalog schema version. Bumped only on breaking (non-additive) changes.
pub const SCHEMA_VERSION: i64 = 2;

/// Catalog DDL, one statement per entry.
pub const DDL_STATEMENTS: &[&str] = &[
    "CREATE TABLE meta (\
       key   TEXT PRIMARY KEY, \
       value TEXT NOT NULL\
     ) WITHOUT ROWID",
    "CREATE TABLE items (\
       unique_name  TEXT PRIMARY KEY, \
       category     TEXT NOT NULL, \
       ducat        INTEGER, \
       wfm_url_name TEXT, \
       tradable     INTEGER NOT NULL DEFAULT 0, \
       icon         TEXT\
     ) WITHOUT ROWID",
    "CREATE TABLE item_names (\
       unique_name TEXT NOT NULL, \
       lang        TEXT NOT NULL, \
       source      TEXT NOT NULL, \
       name        TEXT NOT NULL, \
       PRIMARY KEY (unique_name, lang, source)\
     ) WITHOUT ROWID",
    "CREATE TABLE set_members (\
       set_unique_name    TEXT NOT NULL, \
       member_unique_name TEXT NOT NULL, \
       count              INTEGER NOT NULL DEFAULT 1, \
       PRIMARY KEY (set_unique_name, member_unique_name)\
     ) WITHOUT ROWID",
    "CREATE INDEX idx_item_names_lang_unique ON item_names(lang, unique_name)",
    "CREATE INDEX idx_item_names_lang_name ON item_names(lang, name)",
    "CREATE INDEX idx_set_members_member ON set_members(member_unique_name)",
];
