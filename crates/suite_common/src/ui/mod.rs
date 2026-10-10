//! The suite's shared look: every plugin is a painted stompbox.
//!
//! - `theme`: colours, embedded fonts, text helpers
//! - `widgets`: knob, footswitch, LED, label tape, logo badge
//! - `frame`: the enclosure every editor is drawn on

pub mod frame;
pub mod theme;
pub mod widgets;

pub use frame::{pedal, Ornament, PedalStyle};
pub use theme::install;
pub use widgets::{footswitch, led, param_knob, silk_frame, silk_label};
