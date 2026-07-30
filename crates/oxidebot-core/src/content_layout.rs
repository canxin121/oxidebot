//! Recursive rich-layout content models.

use serde::{Deserialize, Serialize};

use super::{LocationContent, Media, MediaGalleryItem, RichText};
use crate::interaction::{ActionRow, PlatformNativeData};

/// Presentation options for a rich layout node.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LayoutStyle {
    /// Optional RGB accent color.
    pub accent_color: Option<u32>,
    /// Whether content is a spoiler.
    pub spoiler: bool,
    /// Whether the client may collapse the content.
    pub collapsible: bool,
    /// Whether collapsible content starts collapsed.
    pub initially_collapsed: bool,
    /// Lossless platform-specific styling.
    pub platform_data: Option<PlatformNativeData>,
}

/// One column in a multi-column rich layout.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayoutColumn {
    /// Optional relative column width.
    pub width: Option<u16>,
    /// Nodes within the column.
    pub nodes: Vec<LayoutNode>,
}

/// One cell in a rich-layout table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TableCell {
    /// Nodes rendered in the cell.
    pub content: Vec<LayoutNode>,
    /// Whether the cell is a table header.
    pub header: bool,
    /// Number of columns spanned.
    pub colspan: Option<u16>,
    /// Number of rows spanned.
    pub rowspan: Option<u16>,
}

/// One node in a structured cross-platform rich layout.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LayoutNode {
    /// Rich-text node.
    Text(RichText),
    /// Section with child nodes and optional accessory.
    Section {
        /// Main section content.
        children: Vec<LayoutNode>,
        /// Optional side accessory.
        accessory: Option<Box<LayoutNode>>,
        /// Section presentation style.
        style: LayoutStyle,
    },
    /// Generic styled container.
    Container {
        /// Contained nodes.
        children: Vec<LayoutNode>,
        /// Container presentation style.
        style: LayoutStyle,
    },
    /// Multiple layout columns.
    Columns(Vec<LayoutColumn>),
    /// Grid layout.
    Grid {
        /// Number of grid columns.
        columns: u16,
        /// Nodes assigned to the grid.
        children: Vec<LayoutNode>,
    },
    /// Image media.
    Image(Media),
    /// Media gallery.
    MediaGallery(Vec<MediaGalleryItem>),
    /// File media.
    File(Media),
    /// Ordered or unordered list.
    List {
        /// Whether the list is ordered.
        ordered: bool,
        /// Nodes for each list item.
        items: Vec<Vec<LayoutNode>>,
    },
    /// Table layout.
    Table {
        /// Table rows and their cells.
        rows: Vec<Vec<TableCell>>,
    },
    /// Quoted rich text.
    Quote(RichText),
    /// Code block.
    Code {
        /// Source text.
        text: String,
        /// Optional programming language identifier.
        language: Option<String>,
    },
    /// Visual divider.
    Divider,
    /// Geographic map location.
    Map(LocationContent),
    /// Collapsible details block.
    Details {
        /// Always-visible summary.
        summary: RichText,
        /// Detail content.
        children: Vec<LayoutNode>,
        /// Whether details start open.
        open: bool,
    },
    /// Interactive action row.
    Actions(ActionRow),
    /// Lossless platform-native layout node.
    PlatformNative(PlatformNativeData),
}

/// Complete rich layout with optional text fallback.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RichLayout {
    /// Root layout nodes.
    pub nodes: Vec<LayoutNode>,
    /// Text used by platforms that cannot render the layout.
    pub fallback_text: Option<String>,
    /// Lossless platform-specific layout metadata.
    pub platform_data: Option<PlatformNativeData>,
}
