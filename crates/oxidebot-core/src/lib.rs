//! Canonical OxideBot event and API model with the optimized runtime kernel
//! kept behind hidden implementation modules.

pub mod api;
pub mod application;
mod blob;
pub mod bot;
pub mod capability;
pub mod collaboration;
pub mod commerce;
pub mod content;
pub mod conversation;
pub mod event;
pub mod interaction;
pub mod source;

mod id;
#[doc(hidden)]
pub mod message;
mod size;

// Runtime identity and accounting primitives. These are not a second event or
// messaging API; adapters use them to feed the bounded execution kernel.
pub use blob::*;
pub use id::*;
pub use size::*;

pub use api::platform::{
    PlatformApiFile, PlatformApiFileSource, PlatformApiRequest, PlatformApiResponse,
    UnsupportedPlatformApiError,
};
pub use api::CallApiTrait;
pub use application::{
    AppSurface, AppSurfaceKind, BotProfile, CommandChoice, CommandContext, CommandDefinition,
    CommandInvocation, CommandKind, CommandOption, CommandOptionType, Localized, MiniAppEvent,
    MiniAppLaunch, MiniAppMode, Suggestion, SuggestionRequest, SuggestionSelection, SurfaceContent,
    VerificationState,
};
pub use bot::{BotObject, BotTrait};
pub use capability::{
    ApplicationCapabilities, BotCapabilities, ButtonCapabilities, CollaborationCapabilities,
    ComponentCapabilities, ContentCapabilities, ConversationCapabilities, DeliveryCapabilities,
    InteractionLifecycleCapabilities, PlatformLimits, SupportLevel, UnsupportedFeatureError,
};
pub use collaboration::{
    ActivityState, CallKind, CallOptions, CallSession, CallState, ChatActivity, PinOptions,
    PinnedMessage, Reaction, ReactionChange, ReactionOptions, ReactionSummary, ReadReceipt,
};
pub use commerce::{
    CheckoutRequest, CustomerDetails, Invoice, InvoiceOptions, LineItem, Money, Payment,
    PaymentStatus, ShippingOption, ShippingRequest, Subscription,
};
pub use content::{
    BatchItemResult, BatchMessage, BatchSendResult, Checklist, ChecklistChange, ChecklistTask,
    ContactCard, ContentConversionError, CustomEmoji, DeliveryTime, FormValue, ForwardContext,
    ForwardOptions, LayoutColumn, LayoutNode, LayoutStyle, LinkPreviewOptions, LocationContent,
    Media, MediaGalleryItem, MediaType, MentionAllowance, MentionPolicy, MessageContent,
    MessageEnvelope, MessageOrigin, MessageQuery, MessageVisibility, NotificationPolicy,
    OutgoingMessage, PhoneNumber, Poll, PollOption, PollType, ReplyContext, ReplyOptions,
    RichLayout, RichText, Sticker, TableCell, TextSpan, TextStyle,
};
pub use conversation::{
    ConversationKind, ConversationMember, ConversationPermission, ConversationProfile,
    ConversationRef, InviteLink, InviteLinkOptions, JoinRequest, MessageRef, MessageTarget, Page,
    PageRequest, PermissionSet, RoleRef, Thread, ThreadOptions, ThreadState,
};
pub use event::{Event, EventObject, EventTrait};
pub use interaction::{
    ActionRow, BotCommand, BotCommandQuery, BotCommandSet, Button, ButtonAction, ButtonIcon,
    ButtonStyle, ChatMenu, ChoiceOption, ChosenChatCriteria, CommandScope, InlineKeyboard,
    InlineQueryTarget, InputComponent, InteractionCapabilities, InteractionComponent,
    InteractionEvent, InteractionKind, InteractionNotificationStyle, InteractionResponse,
    InteractionResponseHandle, InteractionVisibility, LoginAction, MessageComponents, Modal,
    ModalField, PlatformNativeData, PollKind, ReplyButton, ReplyButtonAction, ReplyButtonRow,
    ReplyKeyboard, RequestChat, RequestManagedBot, RequestUsers, SelectKind, SelectMenu,
    SelectOption, UnsupportedInteractionError, ViewNavigation,
};
pub use source::message::{
    DegradationKind, DeliveryDegradation, DeliveryPlan, DeliveryPlanningError, DeliveryReport,
    FallbackPolicy, File, Folder, FsNode, IntoMessageSegment, Message, MessageOptions,
    MessageSegment, SegmentKind,
};
