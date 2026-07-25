#![doc = include_str!("../Readme.md")]

pub mod api;
pub mod application;
pub mod bot;
pub mod capability;
pub mod collaboration;
pub mod commerce;
pub mod content;
pub mod conversation;
pub mod event;
pub mod filter;
pub mod handler;
pub mod interaction;
pub mod manager;
pub mod matcher;
pub mod source;
pub mod utils;

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
pub use bot::{get_bot, BotTrait};
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
pub use event::EventTrait;
pub use filter::FilterTrait;
pub use handler::ActiveHandlerTrait;
pub use handler::EventHandlerTrait;
pub use handler::Handler;
pub use interaction::{
    ActionRow, BotCommand, BotCommandQuery, BotCommandSet, Button, ButtonAction, ButtonIcon,
    ButtonStyle, ChatMenu, ChoiceOption, ChosenChatCriteria, CommandScope, InlineKeyboard,
    InlineQueryTarget, InputComponent, InteractionCapabilities, InteractionComponent,
    InteractionEvent, InteractionKind, InteractionNotificationStyle, InteractionResponse,
    InteractionResponseHandle, InteractionVisibility, LoginAction, MessageComponents,
    MessageOptions, Modal, ModalField, PlatformNativeData, PollKind, ReplyButton,
    ReplyButtonAction, ReplyButtonRow, ReplyKeyboard, RequestChat, RequestManagedBot, RequestUsers,
    SelectKind, SelectMenu, SelectOption, UnsupportedInteractionError, ViewNavigation,
};
pub use manager::OxideBotManager;
pub use matcher::Matcher;

pub use utils::wait::{
    wait, wait_text_generic, wait_user, wait_user_message, wait_user_text_generic, EasyBool,
};
