use std::path::{Path, PathBuf};
use std::{error::Error, fmt};

use anyhow::{Context as _, Result};
use serde_json::{Map, Value};

/// A lossless request to a platform-native API method.
///
/// This is the extension layer for capabilities that cannot be represented by
/// oxidebot's common, cross-platform methods. Every adapter gets the same API;
/// adapters that do not support native calls keep the default `Unsupported`
/// behavior from [`super::CallApiTrait`].
#[derive(Clone, Debug, PartialEq)]
pub struct PlatformApiRequest {
    pub method: String,
    pub parameters: Value,
    pub files: Vec<PlatformApiFile>,
}

impl PlatformApiRequest {
    pub fn new(method: impl Into<String>) -> Self {
        Self {
            method: method.into(),
            parameters: Value::Object(Map::new()),
            files: Vec::new(),
        }
    }

    pub fn parameters(mut self, parameters: impl Into<Value>) -> Self {
        self.parameters = parameters.into();
        self
    }

    pub fn file(mut self, file: PlatformApiFile) -> Self {
        self.files.push(file);
        self
    }

    pub fn files(mut self, files: impl IntoIterator<Item = PlatformApiFile>) -> Self {
        self.files.extend(files);
        self
    }
}

/// One multipart attachment for a platform-native API request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformApiFile {
    /// Multipart field name. Nested payloads can refer to it using the
    /// platform's attachment syntax, for example Telegram's `attach://name`.
    pub field: String,
    pub file_name: String,
    pub mime_type: Option<String>,
    pub source: PlatformApiFileSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlatformApiFileSource {
    Bytes(Vec<u8>),
    /// Stream this path when the adapter supports streaming multipart bodies.
    Path(PathBuf),
}

impl PlatformApiFile {
    pub fn bytes(
        field: impl Into<String>,
        file_name: impl Into<String>,
        data: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            field: field.into(),
            file_name: file_name.into(),
            mime_type: None,
            source: PlatformApiFileSource::Bytes(data.into()),
        }
    }

    pub fn mime_type(mut self, mime_type: impl Into<String>) -> Self {
        self.mime_type = Some(mime_type.into());
        self
    }

    pub async fn from_path(field: impl Into<String>, path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let metadata = tokio::fs::metadata(path)
            .await
            .with_context(|| format!("failed to inspect platform API attachment {path:?}"))?;
        anyhow::ensure!(metadata.is_file(), "attachment is not a file: {path:?}");
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow::anyhow!("attachment path has no valid file name: {path:?}"))?;
        let mime_type = mime_guess::from_path(path)
            .first()
            .map(|mime| mime.to_string());
        Ok(Self {
            field: field.into(),
            file_name: file_name.to_owned(),
            mime_type,
            source: PlatformApiFileSource::Path(path.to_owned()),
        })
    }
}

/// Lossless platform-native API result.
#[derive(Clone, Debug, PartialEq)]
pub struct PlatformApiResponse {
    pub result: Value,
}

impl PlatformApiResponse {
    pub fn deserialize<T: serde::de::DeserializeOwned>(self) -> Result<T> {
        serde_json::from_value(self.result).context("failed to decode platform API result")
    }
}

/// The adapter does not expose platform-native API calls.
///
/// The default [`super::CallApiTrait::call_platform_api`] implementation
/// returns this concrete error so callers can distinguish an unsupported
/// capability from a transport or remote API failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsupportedPlatformApiError {
    pub method: String,
}

impl fmt::Display for UnsupportedPlatformApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "platform-native API method {:?} is not supported by this adapter",
            self.method
        )
    }
}

impl Error for UnsupportedPlatformApiError {}

#[cfg(test)]
mod tests {
    use crate::api::CallApiTrait;

    use super::*;

    struct UnsupportedAdapter;

    #[async_trait::async_trait]
    impl CallApiTrait for UnsupportedAdapter {}

    #[test]
    fn request_builder_preserves_parameters_and_files() {
        let request = PlatformApiRequest::new("sendPhoto")
            .parameters(serde_json::json!({"chat_id": 42}))
            .file(PlatformApiFile::bytes("photo", "photo.jpg", [1, 2, 3]));
        assert_eq!(request.method, "sendPhoto");
        assert_eq!(request.parameters["chat_id"], 42);
        assert_eq!(
            request.files[0].source,
            PlatformApiFileSource::Bytes(vec![1, 2, 3])
        );
    }

    #[test]
    fn response_can_be_decoded_to_a_caller_type() {
        let response = PlatformApiResponse {
            result: serde_json::json!({"ok": true}),
        };
        let decoded: serde_json::Value = response.deserialize().unwrap();
        assert_eq!(decoded["ok"], true);
    }

    #[tokio::test]
    async fn adapters_get_a_structured_unsupported_error_by_default() {
        let error = UnsupportedAdapter
            .call_platform_api(PlatformApiRequest::new("sendRichMessage"))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            crate::api::CallError::Unsupported { ref feature }
                if feature.contains("sendRichMessage")
        ));
        assert!(!UnsupportedAdapter.supports_platform_api_method("sendRichMessage"));
    }
}
