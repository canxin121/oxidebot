use anyhow::Result;
use hyper::Uri;
use mime::Mime;
use serde_json::Value;
use std::{path::Path, sync::LazyLock};
use tokio::fs::metadata;

use super::user::User;

static REQWESR_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| reqwest::Client::new());

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Message {
    pub id: String,
    pub segments: Vec<MessageSegment>,
}

impl Message {
    // The first text segment is starts with the specified text
    pub fn starts_with_text(&self, text: &str) -> bool {
        self.segments.iter().any(|seg| match seg {
            MessageSegment::Text { content } => content.starts_with(text),
            _ => false,
        })
    }
    
    // Trim the first text segment that starts with the specified text
    pub fn trim_head_text(&self, text: &str) -> Vec<MessageSegment> {
        let mut segments = self.segments.clone();
        for seg in &mut segments {
            if let MessageSegment::Text { content } = seg {
                if content.starts_with(text) {
                    *content = content.trim_start_matches(text).to_string();
                    break;
                }
            }
        }
        segments
    }

    pub fn get_raw_text(&self) -> String {
        self.segments
            .iter()
            .filter_map(|seg| match seg {
                MessageSegment::Text { content } => Some(content.clone()),
                _ => None,
            })
            .collect::<Vec<String>>()
            .join("")
    }

    pub fn is_related_to_user(&self, user_id: &str) -> bool {
        self.segments.iter().any(|seg| match seg {
            MessageSegment::At { user_id: id } => id == user_id,
            MessageSegment::Reply { message_id } => message_id.starts_with(user_id),
            _ => false,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
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

impl MessageSegment {
    pub fn text<T: Into<String>>(content: T) -> Self {
        MessageSegment::Text {
            content: content.into(),
        }
    }

    pub fn image(file: File) -> Self {
        MessageSegment::Image { file: Some(file) }
    }

    pub fn video(file: File, length: Option<i32>) -> Self {
        MessageSegment::Video {
            file: Some(file),
            length,
        }
    }

    pub fn audio(file: File, length: Option<i32>) -> Self {
        MessageSegment::Audio {
            file: Some(file),
            length,
        }
    }

    pub fn file(file: File) -> Self {
        MessageSegment::File { file: Some(file) }
    }

    pub fn reply<T: Into<String>>(message_id: T) -> Self {
        MessageSegment::Reply {
            message_id: message_id.into(),
        }
    }

    pub fn at<T: Into<String>>(user_id: T) -> Self {
        MessageSegment::At {
            user_id: user_id.into(),
        }
    }

    pub fn at_all() -> Self {
        MessageSegment::AtAll
    }

    pub fn reference<T: Into<String>>(message_id: T) -> Self {
        MessageSegment::Reference {
            message_id: message_id.into(),
        }
    }

    pub fn share<T: Into<String>>(
        title: T,
        url: T,
        content: Option<T>,
        image: Option<File>,
    ) -> Self {
        MessageSegment::Share {
            title: title.into(),
            content: content.map(Into::into),
            url: url.into(),
            image,
        }
    }

    pub fn location<T: Into<String>>(
        latitude: f64,
        longitude: f64,
        title: T,
        content: Option<T>,
    ) -> Self {
        MessageSegment::Location {
            latitude,
            longitude,
            title: title.into(),
            content: content.map(Into::into),
        }
    }

    pub fn emoji<T: Into<String>>(id: T) -> Self {
        MessageSegment::Emoji { id: id.into() }
    }

    pub fn forward_node<T: Into<String>>(message_id: T) -> Self {
        MessageSegment::ForwardNode {
            message_id: message_id.into(),
        }
    }

    pub fn forward_custom_node(user: Option<User>, message: Message) -> Self {
        MessageSegment::ForwardCustomNode { user, message }
    }

    pub fn custom_string<T: Into<String>>(r#type: T, data: T) -> Self {
        MessageSegment::CustomString {
            r#type: r#type.into(),
            data: data.into(),
        }
    }

    pub fn custom_value<T: Into<String>>(r#type: T, data: Value) -> Self {
        MessageSegment::CustomValue {
            r#type: r#type.into(),
            data,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct File {
    pub id: Option<String>,
    pub name: String,
    pub uri: Option<Uri>,
    pub base64: Option<String>,
    pub mime: Option<Mime>,
    pub size: Option<u64>,
}

impl File {
    pub async fn try_from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();

        let metadata = metadata(path).await?;
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let size = metadata.len();

        Ok(File {
            id: None,
            name: file_name,
            uri: Some(
                path.to_str()
                    .ok_or(anyhow::anyhow!("invalid path"))?
                    .parse()?,
            ),
            base64: None,
            r#mime: mime_guess::from_path(path).first(),
            size: Some(size),
        })
    }

    pub async fn try_from_url(url: &str) -> Result<Self> {
        let url = url::Url::parse(url)?;
        let file_name = url
            .path_segments()
            .and_then(|segments| segments.last())
            .unwrap_or_default()
            .to_string();

        let response = REQWESR_CLIENT.head(url.clone()).send().await?;
        let size = response
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok());

        let mime = mime_guess::from_path(&file_name).first().or_else(|| {
            response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|ct| ct.to_str().ok())
                .and_then(|ct| ct.parse().ok())
        });

        Ok(File {
            id: None,
            name: file_name,
            uri: Some(url.as_str().parse()?),
            base64: None,
            mime: mime,
            size,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Folder {
    pub id: String,
    pub name: String,
    pub file_amount: u64,
    pub children: Vec<FsNode>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FsNode {
    File(File),
    Folder(Folder),
    Unknown,
}
