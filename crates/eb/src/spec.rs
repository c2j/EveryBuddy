//! Every measured number and every string the shell is built from.
//!
//! Geometry comes from a live CDP capture of the product (see
//! `docs/ui-spec/ui-spec.md` §4.3 for the welcome screen and §4.4 for the
//! task/conversation screen). Keeping the numbers in one file means the layout
//! can be diffed against the capture instead of re-derived by eye.
//!
//! All values are CSS pixels. The capture is at DPR 2, so its device-pixel
//! `bounds` were halved; computed styles were already CSS px and were not
//! touched.
//!
//! Several literals below are recorded but not yet consumed — they document
//! measurements the build does not act on (a gradient size, a pane width that
//! is currently implied by its neighbours). Warning about that would train the
//! reader to ignore warnings.
#![allow(dead_code)]

// Generated metric tokens (spacing/radius/type). Colours live in the generated
// `@define-color` CSS; GTK3 has no numeric variables, so these constants are the
// equivalent for widget spacing, margins and generated CSS.
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/ui-spec/gtk/tokens-metrics.rs"
));

/// Application id — the reverse-DNS name GTK uses for window grouping.
pub const APP_ID: &str = "com.evobuddy.app";
/// Product name. Nothing in the UI may say "WorkBuddy" (see ui-spec §9).
pub const APP_NAME: &str = "EvoBuddy";
pub const APP_VERSION: &str = "v0.1.0";

// ------------------------------------------------------------ window shell

/// Measured: the client area is 1440x868 (the `office` window is 1440x900 minus
/// a 32px macOS title bar).
pub const WINDOW_WIDTH: i32 = 1440;
pub const WINDOW_HEIGHT: i32 = 868;

/// Measured: sidebar 280, conversation pane 580, artifact panel 579, with a 4px
/// sash on each side of the conversation pane.
pub const SIDEBAR_WIDTH: i32 = 280;
pub const CONVERSATION_WIDTH: i32 = 580;
pub const SASH: i32 = spacing::SPACING_2;

/// Measured: with no artifact open the content pane is 1160 wide and the
/// landing column inside it is 800.
pub const CONTENT_WIDTH: i32 = 1160;
pub const COLUMN_WIDTH: i32 = 800;

/// Fixed blocks the CI assertions pin down. The welcome card is the 800-wide
/// column; its 142px height is the implementation value (the product's 144 is
/// ui-spec §10-4). The conversation card's 531 is the measured 532 minus the
/// 1px sash compromise (ui-spec §10-13); the artifact pane stays at 579.
pub const WELCOME_CARD_WIDTH: i32 = COLUMN_WIDTH;
pub const WELCOME_CARD_HEIGHT: i32 = 144;
pub const CHIPS_WIDTH: i32 = COLUMN_WIDTH;
pub const CHIPS_HEIGHT: i32 = 32;
pub const CHAT_CARD_WIDTH: i32 = 531;
/// Product 784 with the artifact panel closed, minus the same 1px sash
/// compromise as the open card.
pub const CHAT_CARD_WIDTH_CLOSED: i32 = 783;
pub const CHAT_CARD_HEIGHT: i32 = 134;
pub const DETAIL_WIDTH: i32 = 579;

/// Measured: with the artifact panel open the conversation pane is 580 and its
/// message column 500 (40px gutters); with the panel closed the column grows to
/// 752 (M2.1, ui-spec §5.3). The artifact pane pads its body by 24px vertically.
pub const MESSAGE_WIDTH: i32 = 500;
pub const MESSAGE_WIDTH_CLOSED: i32 = 752;
pub const MESSAGE_GUTTER: i32 = spacing::SPACING_9;
/// Measured conversation input area (padding 24, card 784) with no artifact;
/// with the panel open the pane itself is 580 and the card 532/531.
pub const CHAT_INPUT_AREA_WIDTH_CLOSED: i32 = 832;

// ------------------------------------------------------------- welcome page

/// Hero top at y=127, content body starting at y=64.
pub const HERO_TOP_GAP: i32 = 63;
/// Hero ends at 169, the segmented control starts at 185.
pub const HERO_TO_SEGMENTED: i32 = 16;
/// Segmented control ends at 221, the composer starts at 285.
pub const SEGMENTED_TO_COMPOSER: i32 = 64;
/// Measured `cr-input-editor-host`.
pub const WELCOME_EDITOR_HEIGHT: i32 = 76;
/// Measured `wb-home-composer__below-input`.
pub const BELOW_INPUT_HEIGHT: i32 = 236;

// -------------------------------------------------------- conversation page

/// Measured `wb-home-route__input` equivalent: the conversation composer's
/// editor is 64 tall inside a 134-tall card, and the AI notice below it is 34.
pub const CHAT_EDITOR_HEIGHT: i32 = 66;
pub const CHAT_INPUT_CARD_HEIGHT: i32 = 134;
pub const CHAT_NOTICE_HEIGHT: i32 = 34;
pub const CHAT_INPUT_AREA_HEIGHT: i32 = 168;
/// Measured `cr-frame` vertical padding around every message.
pub const MESSAGE_FRAME_PAD: i32 = spacing::SPACING_5;

// -------------------------------------------------------------------- copy
//
// The product's own strings are deliberately not reproduced. These stand in at
// roughly the right length so the layout stays verifiable, and are EvoBuddy's
// own wording. Content copy — dropdown menus, pill labels, editor hints, the
// scenario chips, the nav rows, the "not in this build" notices — is
// contributed by plugins (see crates/plugins); what remains here is the shell
// chrome the host renders directly.

pub const HERO: &str = APP_NAME;
pub const EMPTY_TITLE: &str = "No tasks yet";
pub const EMPTY_BODY: &str = "Start a new task and it will show up here.";
pub const NOTICE: &str = "AI-generated content may be inaccurate. Please verify.";
/// Measured: 12px at rgba(0,0,0,.3), centred, 8px padding, in the conversation.
pub const CHAT_NOTICE: &str = "Content generated by AI. Please verify important information.";
