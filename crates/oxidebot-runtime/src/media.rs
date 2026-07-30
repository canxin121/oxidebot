//! Portable media resolution and hosting helpers.
//!
//! Media I/O is independent of command authoring. Keeping it here makes the
//! resolver contract available to messaging and adapter integrations without
//! dragging command-registry implementation details along with it.

use crate::{HandlerError, HandlerResult};
use async_trait::async_trait;
use oxidebot_core::{
    source::message::{File, Message, MessageSegment},
    Media,
};
use std::{path::PathBuf, sync::Arc};

/// In-memory bytes and metadata resolved from a portable file reference.
#[derive(Clone, Debug)]
pub struct ResolvedMedia {
    /// Complete media byte payload.
    pub bytes: Arc<[u8]>,
    /// Detected or declared MIME type, when known.
    pub mime: Option<Arc<str>>,
    /// Suggested filename, when known.
    pub name: Option<Arc<str>>,
}

/// Resolves portable media references into bounded in-memory bytes.
#[async_trait]
pub trait MediaResolver: Send + Sync + 'static {
    /// Resolves the file referenced by a portable media segment.
    async fn resolve(&self, media: &Media) -> HandlerResult<ResolvedMedia>;
    /// Resolves a standalone portable file descriptor.
    async fn resolve_file(&self, file: &File) -> HandlerResult<ResolvedMedia>;
}

/// Resolver for local paths and base64 payloads, bounded by maximum byte size.
#[derive(Clone, Debug)]
pub struct LocalMediaResolver {
    max_bytes: usize,
}

impl LocalMediaResolver {
    /// Creates a local resolver that rejects payloads larger than `max_bytes`.
    #[must_use]
    pub const fn new(max_bytes: usize) -> Self {
        Self { max_bytes }
    }

    pub(crate) async fn read_path(&self, path: PathBuf) -> HandlerResult<ResolvedMedia> {
        use tokio::io::AsyncReadExt;

        let file = tokio::fs::File::open(&path)
            .await
            .map_err(|error| HandlerError::Api(error.to_string()))?;
        let limit = u64::try_from(self.max_bytes.saturating_add(1)).unwrap_or(u64::MAX);
        let mut reader = file.take(limit);
        let mut bytes = Vec::with_capacity(self.max_bytes.min(64 * 1024));
        reader
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| HandlerError::Api(error.to_string()))?;
        if bytes.len() > self.max_bytes {
            return Err(HandlerError::Api(
                "media exceeds configured byte limit".into(),
            ));
        }
        let mime = mime_guess::from_path(&path).first_raw().map(Arc::from);
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .map(Arc::from);
        Ok(ResolvedMedia {
            bytes: bytes.into(),
            mime,
            name,
        })
    }
}

#[async_trait]
impl MediaResolver for LocalMediaResolver {
    async fn resolve(&self, media: &Media) -> HandlerResult<ResolvedMedia> {
        self.resolve_file(&media.file).await
    }

    async fn resolve_file(&self, file: &File) -> HandlerResult<ResolvedMedia> {
        if let Some(path) = &file.path {
            return self.read_path(path.clone()).await;
        }
        if let Some(base64) = &file.base64 {
            if base64.len() > self.max_bytes.saturating_mul(2) {
                return Err(HandlerError::Api(
                    "file exceeds configured byte limit".into(),
                ));
            }
            let bytes = decode_base64(base64)?;
            if bytes.len() > self.max_bytes {
                return Err(HandlerError::Api(
                    "file exceeds configured byte limit".into(),
                ));
            }
            return Ok(ResolvedMedia {
                bytes: bytes.into(),
                mime: file.mime.as_deref().map(Arc::from),
                name: (!file.name.is_empty()).then(|| Arc::from(file.name.as_str())),
            });
        }
        Err(HandlerError::Api(
            "the default media resolver only supports local paths and retained base64 payloads"
                .into(),
        ))
    }
}

fn decode_base64(input: &str) -> HandlerResult<Vec<u8>> {
    fn value(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' | b'-' => Some(62),
            b'/' | b'_' => Some(63),
            _ => None,
        }
    }

    let payload = input
        .strip_prefix("data:")
        .and_then(|value| value.split_once(','))
        .map_or(input, |(_, payload)| payload);
    let bytes = payload
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    if bytes.len() % 4 == 1 {
        return Err(HandlerError::Parse("invalid base64 media payload".into()));
    }
    let mut output = Vec::with_capacity(bytes.len().saturating_mul(3) / 4);
    let mut index = 0;
    while index < bytes.len() {
        let remaining = bytes.len() - index;
        let take = remaining.min(4);
        let chunk = &bytes[index..index + take];
        let padding = chunk.iter().rev().take_while(|byte| **byte == b'=').count();
        if padding > 2
            || (padding > 0 && index + take != bytes.len())
            || chunk[..chunk.len().saturating_sub(padding)].contains(&b'=')
        {
            return Err(HandlerError::Parse("invalid base64 media payload".into()));
        }
        let mut values = [0_u8; 4];
        for (slot, byte) in chunk.iter().enumerate() {
            if *byte != b'=' {
                values[slot] = value(*byte)
                    .ok_or_else(|| HandlerError::Parse("invalid base64 media payload".into()))?;
            }
        }
        output.push((values[0] << 2) | (values[1] >> 4));
        if take >= 3 && padding < 2 {
            output.push((values[1] << 4) | (values[2] >> 2));
        }
        if take == 4 && padding == 0 {
            output.push((values[2] << 6) | values[3]);
        }
        index += take;
    }
    Ok(output)
}

/// One resolved file-bearing message segment and its structural path.
#[derive(Clone, Debug)]
pub struct ResolvedMessageMedia {
    /// Structural path identifying the file-bearing segment in the message.
    pub path: Arc<str>,
    /// Resolved byte payload and metadata for that segment.
    pub media: ResolvedMedia,
}

/// Resolves every file-bearing segment, including nested forwarded messages,
/// gallery items, share images, captions' thumbnails, and rich media.
pub async fn resolve_message_media<R>(
    resolver: &R,
    message: &Message,
) -> HandlerResult<Vec<ResolvedMessageMedia>>
where
    R: MediaResolver + ?Sized,
{
    enum Source<'a> {
        File(&'a File),
        Media(&'a Media),
    }
    fn collect<'a>(message: &'a Message, prefix: &str, output: &mut Vec<(String, Source<'a>)>) {
        for (index, segment) in message.segments.iter().enumerate() {
            let path = format!("{prefix}segments[{index}]");
            match segment {
                MessageSegment::Share {
                    image: Some(file), ..
                } => {
                    output.push((format!("{path}.image"), Source::File(file)));
                }
                MessageSegment::Media { media, .. } => {
                    output.push((path.clone(), Source::Media(media)));
                    if let Some(thumbnail) = &media.thumbnail {
                        output.push((format!("{path}.thumbnail"), Source::File(thumbnail)));
                    }
                }
                MessageSegment::MediaGallery(items) => {
                    for (item_index, item) in items.iter().enumerate() {
                        let item_path = format!("{path}.items[{item_index}]");
                        output.push((item_path.clone(), Source::Media(&item.media)));
                        if let Some(thumbnail) = &item.media.thumbnail {
                            output
                                .push((format!("{item_path}.thumbnail"), Source::File(thumbnail)));
                        }
                    }
                }
                MessageSegment::Emoji(emoji) => {
                    if let Some(file) = &emoji.file {
                        output.push((path, Source::File(file)));
                    }
                }
                MessageSegment::Sticker(sticker) => {
                    if let Some(file) = &sticker.file {
                        output.push((path, Source::File(file)));
                    }
                }
                MessageSegment::ForwardCustomNode { message, .. } => {
                    collect(message, &format!("{path}.message."), output);
                }
                _ => {}
            }
        }
    }

    let mut sources = Vec::new();
    collect(message, "", &mut sources);
    let mut output = Vec::with_capacity(sources.len());
    for (path, source) in sources {
        let media = match source {
            Source::File(file) => resolver.resolve_file(file).await?,
            Source::Media(media) => resolver.resolve(media).await?,
        };
        output.push(ResolvedMessageMedia {
            path: Arc::from(path),
            media,
        });
    }
    Ok(output)
}

/// Stores resolved media where adapters can later retrieve it.
#[async_trait]
pub trait MediaHost: Send + Sync + 'static {
    /// Stores resolved bytes and returns a portable file descriptor, usually
    /// containing a URL accepted by adapters that cannot upload local bytes.
    async fn host(&self, media: ResolvedMedia) -> HandlerResult<File>;
}

/// Fetches remote media under an application-defined network policy.
#[async_trait]
pub trait MediaFetcher: Send + Sync + 'static {
    /// Fetches one external URI under the caller's byte and protocol policy.
    async fn fetch(&self, uri: &str, max_bytes: usize) -> HandlerResult<ResolvedMedia>;
}

/// Media resolver that uses local data first and delegates URIs to a fetcher.
#[derive(Clone)]
pub struct PortableMediaResolver<F> {
    local: LocalMediaResolver,
    fetcher: Arc<F>,
}

impl<F> PortableMediaResolver<F> {
    /// Creates a resolver with a bounded local resolver and remote `fetcher`.
    #[must_use]
    pub fn new(max_bytes: usize, fetcher: F) -> Self {
        Self {
            local: LocalMediaResolver::new(max_bytes),
            fetcher: Arc::new(fetcher),
        }
    }
}

#[async_trait]
impl<F> MediaResolver for PortableMediaResolver<F>
where
    F: MediaFetcher,
{
    async fn resolve(&self, media: &Media) -> HandlerResult<ResolvedMedia> {
        self.resolve_file(&media.file).await
    }

    async fn resolve_file(&self, file: &File) -> HandlerResult<ResolvedMedia> {
        if file.path.is_some() || file.base64.is_some() {
            return self.local.resolve_file(file).await;
        }
        let uri = file.uri.as_deref().ok_or_else(|| {
            HandlerError::Api("file has no resolvable path, payload, or URI".into())
        })?;
        self.fetcher.fetch(uri, self.local.max_bytes).await
    }
}

/// Resolves `file` and stores the resulting bytes through `host`.
pub async fn resolve_and_host_file<R, H>(resolver: &R, host: &H, file: &File) -> HandlerResult<File>
where
    R: MediaResolver + ?Sized,
    H: MediaHost + ?Sized,
{
    host.host(resolver.resolve_file(file).await?).await
}
