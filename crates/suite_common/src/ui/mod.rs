//! The suite's shared look: every plugin is a painted stompbox.
//!
//! - `theme`: colours, embedded fonts, text helpers
//! - `widgets`: knob, footswitch, LED, label tape, stepper, tuner display, logo badge
//! - `frame`: the enclosure every editor is drawn on. `pedal` is one stompbox filling the window;
//!   `rig` is the bare bench for a `head` with `mini_pedal`s below it

pub mod frame;
pub mod theme;
pub mod widgets;

pub use frame::{enclosure, head, mini_pedal, pedal, rig, Ornament, PedalStyle, BENCH_MARGIN};
pub use theme::install;
pub use widgets::{
    cents_meter, footswitch, led, param_knob, param_switch, rig_led, silk_frame, silk_label, small_footswitch, stepper,
    tuner_display, Step,
};
