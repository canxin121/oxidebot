//! Portable file reference model used by messages and rich content.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Pure attachment descriptor. Construction never performs I/O; adapters own
/// path inspection, URL probing, streaming, and upload policy.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct File {
    /// Platform-assigned file identifier, if available.
    pub id: Option<String>,
    /// User-visible file name.
    pub name: String,
    /// Remote URI from which an adapter may fetch the file.
    pub uri: Option<String>,
    /// Local filesystem path from which an adapter may read the file.
    pub path: Option<PathBuf>,
    /// Inline base64-encoded file content.
    pub base64: Option<String>,
    /// MIME type, if known.
    pub mime: Option<String>,
    /// File size in bytes, if known.
    pub size: Option<u64>,
}

impl File {
    /// Creates a local file descriptor after validating and inspecting `path`.
    pub async fn try_from_path<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let metadata = tokio::fs::metadata(path).await?;
        anyhow::ensure!(metadata.is_file(), "path does not point to a file");

        let mut file = Self::from_path(path).size(metadata.len());
        file.mime = mime_guess::from_path(path)
            .first()
            .map(|mime| mime.to_string());
        Ok(file)
    }

    /// Creates a remote file descriptor after validating the URL shape.
    pub fn try_from_url(url: &str) -> anyhow::Result<Self> {
        anyhow::ensure!(url.contains("://"), "invalid URL");
        Ok(Self::from_url(url))
    }

    /// Creates a descriptor for a platform-owned file identifier.
    #[must_use]
    pub fn platform(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: Some(id.into()),
            name: name.into(),
            ..Self::default()
        }
    }

    /// Creates a local path-backed descriptor without performing I/O.
    #[must_use]
    pub fn from_path(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        Self {
            name: path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_owned(),
            path: Some(path.to_owned()),
            ..Self::default()
        }
    }

    /// Creates a remote URL-backed descriptor without performing I/O.
    #[must_use]
    pub fn from_url(url: impl Into<String>) -> Self {
        let url = url.into();
        let name = url
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .split('?')
            .next()
            .unwrap_or_default()
            .to_owned();
        Self {
            name,
            uri: Some(url),
            ..Self::default()
        }
    }

    /// Sets the MIME type.
    #[must_use]
    pub fn mime(mut self, mime: impl Into<String>) -> Self {
        self.mime = Some(mime.into());
        self
    }

    /// Sets the known byte size.
    #[must_use]
    pub fn size(mut self, size: u64) -> Self {
        self.size = Some(size);
        self
    }

    /// Estimates bytes retained by this descriptor.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.id.as_ref().map_or(0, String::len)
            + self.name.len()
            + self.uri.as_ref().map_or(0, String::len)
            + self
                .path
                .as_ref()
                .map_or(0, |path| path.as_os_str().as_encoded_bytes().len())
            + self.base64.as_ref().map_or(0, String::len)
            + self.mime.as_ref().map_or(0, String::len)
            + 64
    }
}
