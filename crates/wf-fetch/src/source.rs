use crate::de_export::{IndexEntry, fetch_index, fetch_manifest};

/// Default DE Public Export index endpoint (CDN host — reachable from datacenter IPs).
pub const DE_INDEX_BASE: &str = "https://content.warframe.com/PublicExport/index_";
/// Default DE Public Export manifest endpoint.
pub const DE_MANIFEST_BASE: &str = "https://content.warframe.com/PublicExport/Manifest/";

/// A source of DE-style catalog manifests.
pub trait ManifestSource {
    /// Fetch the parsed index for a language.
    fn fetch_index(&self, lang: &str) -> anyhow::Result<Vec<IndexEntry>>;
    /// Fetch and parse one manifest referenced by an index entry.
    fn fetch_manifest(&self, entry: &IndexEntry) -> anyhow::Result<serde_json::Value>;
}

/// DE Public Export source over HTTP with configurable endpoints.
pub struct DeExportSource {
    agent: ureq::Agent,
    index_base: String,
    manifest_base: String,
}

impl DeExportSource {
    /// Build a source from explicit endpoints.
    pub fn new(
        agent: ureq::Agent,
        index_base: impl Into<String>,
        manifest_base: impl Into<String>,
    ) -> Self {
        Self {
            agent,
            index_base: index_base.into(),
            manifest_base: manifest_base.into(),
        }
    }

    /// Build a source with the default DE CDN endpoints.
    pub fn with_defaults(agent: ureq::Agent) -> Self {
        Self::new(agent, DE_INDEX_BASE, DE_MANIFEST_BASE)
    }
}

impl ManifestSource for DeExportSource {
    fn fetch_index(&self, lang: &str) -> anyhow::Result<Vec<IndexEntry>> {
        fetch_index(&self.agent, &self.index_base, lang)
    }

    fn fetch_manifest(&self, entry: &IndexEntry) -> anyhow::Result<serde_json::Value> {
        fetch_manifest(&self.agent, &self.manifest_base, entry)
    }
}
