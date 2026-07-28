use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

use super::user::User;

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub segments: Vec<MessageSegment>,
}

impl Message {
    /// Creates an outgoing message containing one text segment. The empty
    /// `id` marks a message that has not been sent yet.
    #[must_use]
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            id: String::new(),
            segments: vec![MessageSegment::text(content)],
        }
    }

    /// Creates an outgoing message from canonical 0.1.8 message segments.
    #[must_use]
    pub fn from_segments(segments: impl IntoIterator<Item = MessageSegment>) -> Self {
        Self {
            id: String::new(),
            segments: segments.into_iter().collect(),
        }
    }

    /// Appends one segment-like value and returns the message for fluent use.
    #[must_use]
    pub fn then(mut self, segment: impl IntoMessageSegment) -> Self {
        self.push(segment);
        self
    }

    /// Appends one segment-like value in place.
    pub fn push(&mut self, segment: impl IntoMessageSegment) {
        self.segments.push(segment.into_message_segment());
    }

    #[must_use]
    pub fn at(self, user_id: impl Into<String>) -> Self {
        self.then(MessageSegment::at(user_id))
    }

    #[must_use]
    pub fn at_all(self) -> Self {
        self.then(MessageSegment::at_all())
    }

    #[must_use]
    pub fn reply_to(self, message_id: impl Into<String>) -> Self {
        self.then(MessageSegment::reply(message_id))
    }

    #[must_use]
    pub fn reference(self, message_id: impl Into<String>) -> Self {
        self.then(MessageSegment::reference(message_id))
    }

    #[must_use]
    pub fn image(self, file: File) -> Self {
        self.then(MessageSegment::image(file))
    }

    #[must_use]
    pub fn video(self, file: File, length: Option<i32>) -> Self {
        self.then(MessageSegment::video(file, length))
    }

    #[must_use]
    pub fn audio(self, file: File, length: Option<i32>) -> Self {
        self.then(MessageSegment::audio(file, length))
    }

    #[must_use]
    pub fn file(self, file: File) -> Self {
        self.then(MessageSegment::file(file))
    }

    #[must_use]
    pub fn emoji(self, id: impl Into<String>) -> Self {
        self.then(MessageSegment::emoji(id))
    }

    #[must_use]
    pub fn into_segments(self) -> Vec<MessageSegment> {
        self.segments
    }

    /// Returns whether any text segment starts with `text`.
    #[must_use]
    pub fn starts_with_text(&self, text: &str) -> bool {
        self.segments.iter().any(|segment| match segment {
            MessageSegment::Text { content } => content.starts_with(text),
            _ => false,
        })
    }

    /// Clones the segments and trims the first matching text prefix.
    #[must_use]
    pub fn trim_head_text(&self, text: &str) -> Vec<MessageSegment> {
        let mut segments = self.segments.clone();
        for segment in &mut segments {
            if let MessageSegment::Text { content } = segment {
                if content.starts_with(text) {
                    *content = content.trim_start_matches(text).to_owned();
                    break;
                }
            }
        }
        segments
    }

    /// Concatenates all plain-text segments without allocating intermediate strings.
    #[must_use]
    pub fn get_raw_text(&self) -> String {
        let capacity = self
            .segments
            .iter()
            .map(|segment| match segment {
                MessageSegment::Text { content } => content.len(),
                _ => 0,
            })
            .sum();
        let mut output = String::with_capacity(capacity);
        for segment in &self.segments {
            if let MessageSegment::Text { content } = segment {
                output.push_str(content);
            }
        }
        output
    }

    #[must_use]
    pub fn is_related_to_user(&self, user_id: &str) -> bool {
        self.segments.iter().any(|segment| match segment {
            MessageSegment::At { user_id: id } => id == user_id,
            MessageSegment::Reply { message_id } => message_id.starts_with(user_id),
            _ => false,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MessageSegment {
    Text {
        content: String,
    },
    Image {
        file: Option<File>,
    },
    Video {
        file: Option<File>,
        length: Option<i32>,
    },
    Audio {
        file: Option<File>,
        length: Option<i32>,
    },
    File {
        file: Option<File>,
    },
    Reply {
        message_id: String,
    },
    At {
        user_id: String,
    },
    AtAll,
    Reference {
        message_id: String,
    },
    Share {
        title: String,
        content: Option<String>,
        url: String,
        image: Option<File>,
    },
    Location {
        latitude: f64,
        longitude: f64,
        title: String,
        content: Option<String>,
    },
    Emoji {
        id: String,
    },
    ForwardNode {
        message_id: String,
    },
    ForwardCustomNode {
        user: Option<User>,
        message: Message,
    },
    CustomString {
        r#type: String,
        data: String,
    },
    CustomValue {
        r#type: String,
        data: Value,
    },
}

/// Converts a value into one canonical 0.1.8 message segment.
pub trait IntoMessageSegment {
    fn into_message_segment(self) -> MessageSegment;
}

impl IntoMessageSegment for MessageSegment {
    fn into_message_segment(self) -> MessageSegment {
        self
    }
}

impl IntoMessageSegment for String {
    fn into_message_segment(self) -> MessageSegment {
        MessageSegment::text(self)
    }
}

impl IntoMessageSegment for &str {
    fn into_message_segment(self) -> MessageSegment {
        MessageSegment::text(self)
    }
}

impl IntoMessageSegment for &String {
    fn into_message_segment(self) -> MessageSegment {
        MessageSegment::text(self.clone())
    }
}

impl From<MessageSegment> for Message {
    fn from(segment: MessageSegment) -> Self {
        Self::from_segments([segment])
    }
}

impl From<Vec<MessageSegment>> for Message {
    fn from(segments: Vec<MessageSegment>) -> Self {
        Self {
            id: String::new(),
            segments,
        }
    }
}

impl From<String> for Message {
    fn from(text: String) -> Self {
        Self::text(text)
    }
}

impl From<&str> for Message {
    fn from(text: &str) -> Self {
        Self::text(text)
    }
}

impl From<&String> for Message {
    fn from(text: &String) -> Self {
        Self::text(text.clone())
    }
}

impl From<Message> for Vec<MessageSegment> {
    fn from(message: Message) -> Self {
        message.segments
    }
}

impl MessageSegment {
    #[must_use]
    pub fn text(content: impl Into<String>) -> Self {
        Self::Text {
            content: content.into(),
        }
    }

    #[must_use]
    pub fn image(file: File) -> Self {
        Self::Image { file: Some(file) }
    }

    #[must_use]
    pub fn video(file: File, length: Option<i32>) -> Self {
        Self::Video {
            file: Some(file),
            length,
        }
    }

    #[must_use]
    pub fn audio(file: File, length: Option<i32>) -> Self {
        Self::Audio {
            file: Some(file),
            length,
        }
    }

    #[must_use]
    pub fn file(file: File) -> Self {
        Self::File { file: Some(file) }
    }

    #[must_use]
    pub fn reply(message_id: impl Into<String>) -> Self {
        Self::Reply {
            message_id: message_id.into(),
        }
    }

    #[must_use]
    pub fn at(user_id: impl Into<String>) -> Self {
        Self::At {
            user_id: user_id.into(),
        }
    }

    #[must_use]
    pub const fn at_all() -> Self {
        Self::AtAll
    }

    #[must_use]
    pub fn reference(message_id: impl Into<String>) -> Self {
        Self::Reference {
            message_id: message_id.into(),
        }
    }

    #[must_use]
    pub fn share<T: Into<String>>(
        title: T,
        url: T,
        content: Option<T>,
        image: Option<File>,
    ) -> Self {
        Self::Share {
            title: title.into(),
            content: content.map(Into::into),
            url: url.into(),
            image,
        }
    }

    #[must_use]
    pub fn location<T: Into<String>>(
        latitude: f64,
        longitude: f64,
        title: T,
        content: Option<T>,
    ) -> Self {
        Self::Location {
            latitude,
            longitude,
            title: title.into(),
            content: content.map(Into::into),
        }
    }

    #[must_use]
    pub fn emoji(id: impl Into<String>) -> Self {
        Self::Emoji { id: id.into() }
    }

    #[must_use]
    pub fn forward_node(message_id: impl Into<String>) -> Self {
        Self::ForwardNode {
            message_id: message_id.into(),
        }
    }

    #[must_use]
    pub fn forward_custom_node(user: Option<User>, message: Message) -> Self {
        Self::ForwardCustomNode { user, message }
    }

    #[must_use]
    pub fn custom_string<T: Into<String>>(r#type: T, data: T) -> Self {
        Self::CustomString {
            r#type: r#type.into(),
            data: data.into(),
        }
    }

    #[must_use]
    pub fn custom_value(r#type: impl Into<String>, data: Value) -> Self {
        Self::CustomValue {
            r#type: r#type.into(),
            data,
        }
    }
}

/// Pure attachment descriptor. Construction never performs I/O; adapters own
/// path inspection, URL probing, streaming, and upload policy.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct File {
    pub id: Option<String>,
    pub name: String,
    pub uri: Option<String>,
    pub path: Option<PathBuf>,
    pub base64: Option<String>,
    pub mime: Option<String>,
    pub size: Option<u64>,
}

impl File {
    /// Builds a file descriptor from a local path.
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

    /// Builds a file descriptor from a URL.
    /// Network probing remains adapter-owned so constructing an event never
    /// performs hidden I/O.
    pub async fn try_from_url(url: &str) -> anyhow::Result<Self> {
        anyhow::ensure!(url.contains("://"), "invalid URL");
        Ok(Self::from_url(url))
    }

    #[must_use]
    pub fn platform(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: Some(id.into()),
            name: name.into(),
            ..Self::default()
        }
    }

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

    #[must_use]
    pub fn mime(mut self, mime: impl Into<String>) -> Self {
        self.mime = Some(mime.into());
        self
    }

    #[must_use]
    pub fn size(mut self, size: u64) -> Self {
        self.size = Some(size);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Folder {
    pub id: String,
    pub name: String,
    pub file_amount: u64,
    pub children: Vec<FsNode>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FsNode {
    File(File),
    Folder(Folder),
    Unknown,
}
