//! The suite's shared look: every plugin is a painted stompbox.
//!
//! - `theme`: colours, embedded fonts, text helpers
//! - `widgets`: knob, footswitch, LED, signal lamp, label tape, stepper, tuner display, logo badge
//! - `art`: what is painted on each mini pedal of a rig under its controls
//! - `frame`: the enclosure every editor is drawn on. `pedal` is one stompbox filling the window;
//!   `rig` is the bare bench for a `head` with `mini_pedal`s and `slim_pedal`s below it

pub mod art;
pub mod frame;
pub mod theme;
pub mod widgets;

pub use art::Motif;
pub use frame::{
    amp_marker, enclosure, head, lifted, mini_pedal, pedal, rig, slim_pedal, title_grip, MiniStyle, Ornament,
    PedalStyle,
    BENCH_MARGIN, SLIM_KNOBS, SLIM_KNOB_RADIUS, SLIM_LED, SLIM_PEDAL_WIDTH, SLIM_SWITCH,
};
pub use theme::install;
pub use widgets::{
    cents_meter, footswitch, led, param_knob, param_switch, rig_led, signal_lamp, silk_frame, silk_label,
    small_footswitch, stepper, tuner_display, value_knob, value_switch, Step,
};
