//! The pedalboard: one row of pedals under the head, in the order the signal goes through
//! them, with the amp's place marked between the pedals in front of it and the cabinet. A
//! pedal is moved by dragging it by its title: the others make room, and it lands in the
//! nearest place it may have.
//!
//! The order itself is `order::ChainOrder`, kept with the parameters and saved with the
//! project. What lives in the window is only where the pedals are drawn while one is held
//! or gliding to its place.

use nih_plug_egui::egui::{vec2, Id, Pos2, Rect, Ui};
use suite_common::ui::{self, BENCH_MARGIN, SLIM_PEDAL_WIDTH};

use crate::editor::{
    Pedals, CAB_SLOT, GATE_SLOT, HEAD_HEIGHT, PEDAL_HEIGHT, PEDAL_TOP, PEDAL_WIDTH,
};
use crate::order::{self, ChainOrder};

const PEDALS: usize = 10;
// The bench between the last pedal in front of the amp and the cabinet, where the amp's
// marker is
const AMP_GAP: f32 = 40.0;
// Five pedals of 180, five of 110, a margin on both sides of each, and the amp's gap in
// the place of one margin
pub const WINDOW_WIDTH: u32 = 1590;

// Share of its way to its place that a pedal making room goes in a second, were it to keep its speed
const GLIDE_PER_SECOND: f32 = 18.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pedal {
    Gate,
    Comp,
    Drive,
    Phaser,
    Cab,
    Chorus,
    Flanger,
    Tremolo,
    Delay,
    Reverb,
}

/// Where in the chain a pedal may be
#[derive(PartialEq)]
enum Zone {
    /// Where it is: the gate first, the cabinet right behind the amp
    Fixed,
    /// In front of the amp
    Front,
    /// Behind the cabinet
    Back,
    Either,
}

impl Pedal {
    fn zone(self) -> Zone {
        match self {
            Pedal::Gate | Pedal::Cab => Zone::Fixed,
            Pedal::Drive => Zone::Front,
            Pedal::Delay | Pedal::Reverb => Zone::Back,
            Pedal::Comp | Pedal::Phaser | Pedal::Chorus | Pedal::Flanger | Pedal::Tremolo => {
                Zone::Either
            }
        }
    }

    /// The pedal's number in the editor's list of pedals
    fn slot(self) -> usize {
        match self {
            Pedal::Gate => GATE_SLOT,
            Pedal::Drive => 1,
            Pedal::Cab => CAB_SLOT,
            Pedal::Delay => 3,
            Pedal::Reverb => 4,
            Pedal::Comp => 5,
            Pedal::Chorus => 6,
            Pedal::Phaser => 7,
            Pedal::Flanger => 8,
            Pedal::Tremolo => 9,
        }
    }

    fn width(self) -> f32 {
        if Pedals::is_slim(self.slot()) {
            SLIM_PEDAL_WIDTH
        } else {
            PEDAL_WIDTH
        }
    }

    /// What the chain calls the pedal. None for the two that are no part of its order
    fn in_chain(self) -> Option<order::Pedal> {
        match self {
            Pedal::Gate | Pedal::Cab => None,
            Pedal::Comp => Some(order::Pedal::Comp),
            Pedal::Drive => Some(order::Pedal::Drive),
            Pedal::Phaser => Some(order::Pedal::Phaser),
            Pedal::Chorus => Some(order::Pedal::Chorus),
            Pedal::Flanger => Some(order::Pedal::Flanger),
            Pedal::Tremolo => Some(order::Pedal::Tremolo),
            Pedal::Delay => Some(order::Pedal::Delay),
            Pedal::Reverb => Some(order::Pedal::Reverb),
        }
    }

    fn of_chain(pedal: order::Pedal) -> Pedal {
        match pedal {
            order::Pedal::Comp => Pedal::Comp,
            order::Pedal::Drive => Pedal::Drive,
            order::Pedal::Phaser => Pedal::Phaser,
            order::Pedal::Chorus => Pedal::Chorus,
            order::Pedal::Flanger => Pedal::Flanger,
            order::Pedal::Tremolo => Pedal::Tremolo,
            order::Pedal::Delay => Pedal::Delay,
            order::Pedal::Reverb => Pedal::Reverb,
        }
    }
}

/// The pedals in signal order. The first `front` of them are in front of the amp, and the
/// cabinet is the first of the others
#[derive(Clone, Copy, Debug, PartialEq)]
struct Order {
    pedals: [Pedal; PEDALS],
    front: usize,
}

impl Default for Order {
    fn default() -> Self {
        use Pedal::*;
        Self {
            pedals: [Gate, Comp, Drive, Phaser, Cab, Chorus, Flanger, Tremolo, Delay, Reverb],
            front: 4,
        }
    }
}

impl Order {
    /// The row the chain's order makes: the gate, the pedals in front of the amp, the
    /// cabinet, the pedals behind it
    fn of_chain(chain: &ChainOrder) -> Order {
        let front = chain.front().iter().map(|&pedal| Pedal::of_chain(pedal));
        let back = chain.back().iter().map(|&pedal| Pedal::of_chain(pedal));
        let row = std::iter::once(Pedal::Gate)
            .chain(front)
            .chain(std::iter::once(Pedal::Cab))
            .chain(back);
        let mut pedals = [Pedal::Gate; PEDALS];
        for (place, pedal) in pedals.iter_mut().zip(row) {
            *place = pedal;
        }
        Order { pedals, front: chain.front().len() + 1 }
    }

    /// The chain's order for this row
    fn chain(&self) -> ChainOrder {
        let ids =
            self.pedals.iter().enumerate().filter_map(|(place, pedal)| match pedal.in_chain() {
                Some(pedal) => Some(pedal.id()),
                None => (place == self.front).then_some(order::AMP_ID),
            });
        ChainOrder::from_ids(ids)
    }

    /// The left edge of every pedal (by `Pedal as usize`) and of the amp's gap, from the
    /// left edge of the window
    fn places(&self) -> ([f32; PEDALS], f32) {
        let mut lefts = [0.0; PEDALS];
        let mut gap = 0.0;
        let mut x = BENCH_MARGIN;
        for (position, &pedal) in self.pedals.iter().enumerate() {
            if position == self.front {
                gap = x - BENCH_MARGIN;
                x = gap + AMP_GAP;
            }
            lefts[pedal as usize] = x;
            x += pedal.width() + BENCH_MARGIN;
        }
        (lefts, gap)
    }

    /// Every order with `pedal` in a place it may have, this one included, and the others
    /// as they follow each other now. None for a pedal that is fixed
    fn moves(&self, pedal: Pedal) -> Vec<Order> {
        let Some(was) = self.pedals.iter().position(|&other| other == pedal) else {
            return Vec::new();
        };
        let mut others = self.pedals.to_vec();
        others.remove(was);
        // The cabinet is at this place among the others
        let front = self.front - usize::from(was < self.front);

        let mut found = Vec::new();
        // Never in front of the gate
        for at in 1..=others.len() {
            let in_front = at <= front;
            let allowed = match pedal.zone() {
                Zone::Fixed => false,
                Zone::Front => in_front,
                Zone::Back => !in_front,
                Zone::Either => true,
            };
            if allowed {
                let mut pedals = self.pedals;
                pedals[..at].copy_from_slice(&others[..at]);
                pedals[at] = pedal;
                pedals[at + 1..].copy_from_slice(&others[at..]);
                found.push(Order { pedals, front: front + usize::from(in_front) });
            }
        }
        found
    }

    /// The order `pedal` would land in if it were let go with its left edge at `left`: of
    /// the places it may have, the one nearest to there
    fn nearest(&self, pedal: Pedal, left: f32) -> Order {
        let distance = |order: &Order| (order.places().0[pedal as usize] - left).abs();
        self.moves(pedal)
            .into_iter()
            .min_by(|a, b| distance(a).total_cmp(&distance(b)))
            .unwrap_or(*self)
    }
}

/// The pedal that is held by its title
#[derive(Clone, Copy)]
struct Held {
    pedal: Pedal,
    /// From the pedal's left edge to where the pointer took it
    grip: f32,
    /// Its left edge, from the left edge of the window
    left: f32,
}

/// What the window keeps of the board from frame to frame
#[derive(Clone)]
struct Board {
    held: Option<Held>,
    /// Where every pedal and the amp's gap are drawn now: they glide to their places
    lefts: [f32; PEDALS],
    gap: f32,
}

impl Board {
    /// Every pedal in its place
    fn at_rest(order: &Order) -> Self {
        let (lefts, gap) = order.places();
        Self { held: None, lefts, gap }
    }
}

/// The whole row under the head, in the order `chain` holds, which a pedal that is dragged
/// and let go changes. `window` is the window's top left corner; `pedals` draws each pedal
pub fn board(ui: &mut Ui, window: Pos2, pedals: &Pedals, chain: &order::StoredOrder) {
    // Read every frame: a preset or the host may have changed it
    let order = Order::of_chain(&chain.get());
    let id = Id::new("board");
    let mut board: Board =
        ui.data(|data| data.get_temp(id)).unwrap_or_else(|| Board::at_rest(&order));
    let held = board.held.map(|held| held.pedal);

    // The others stand where they would if the held pedal were let go now
    let shown = board.held.map_or(order, |held| order.nearest(held.pedal, held.left));
    let (places, gap_place) = shown.places();
    let step = (ui.input(|input| input.stable_dt) * GLIDE_PER_SECOND).min(1.0);
    let mut gliding = false;
    let mut glide = |at: &mut f32, to: f32| {
        if (to - *at).abs() < 0.5 {
            *at = to;
        } else {
            *at += (to - *at) * step;
            gliding = true;
        }
    };
    for pedal in shown.pedals {
        if Some(pedal) != held {
            glide(&mut board.lefts[pedal as usize], places[pedal as usize]);
        }
    }
    glide(&mut board.gap, gap_place);
    if gliding {
        ui.ctx().request_repaint();
    }

    let gap = Rect::from_min_size(window + vec2(board.gap, PEDAL_TOP), vec2(AMP_GAP, PEDAL_HEIGHT));
    ui::amp_marker(ui.painter(), gap, window.y + BENCH_MARGIN + HEAD_HEIGHT);

    // The held pedal last, so it is above the others
    let in_turn = shown.pedals.into_iter().filter(|&pedal| Some(pedal) != held).chain(held);
    for pedal in in_turn {
        let left = board.lefts[pedal as usize];
        let mut rect =
            Rect::from_min_size(window + vec2(left, PEDAL_TOP), vec2(pedal.width(), PEDAL_HEIGHT));
        if Some(pedal) == held {
            rect = ui::lifted(ui.painter(), rect);
        }
        let face = pedals.pedal(ui, pedal.slot(), rect);
        if pedal.zone() == Zone::Fixed {
            continue;
        }

        let grip = ui::title_grip(ui, face, &format!("{pedal:?}"));
        if grip.drag_started() {
            // Where the button went down, not where the drag was noticed
            let taken =
                ui.input(|input| input.pointer.press_origin()).map_or(face.center().x, |pos| pos.x);
            board.held = Some(Held { pedal, grip: taken - rect.left(), left });
        }
        if let (Some(held), Some(pointer)) = (&mut board.held, grip.interact_pointer_pos()) {
            // Not over the gate and not out of the window
            let leftmost = BENCH_MARGIN + PEDAL_WIDTH + BENCH_MARGIN;
            let rightmost = WINDOW_WIDTH as f32 - BENCH_MARGIN - pedal.width();
            held.left = (pointer.x - window.x - held.grip).clamp(leftmost, rightmost);
            board.lefts[pedal as usize] = held.left;
        }
        if grip.drag_stopped() {
            if let Some(held) = board.held.take() {
                chain.put(order.nearest(held.pedal, held.left).chain());
            }
        }
    }

    ui.data_mut(|data| data.insert_temp(id, board));
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOVABLE: [Pedal; 8] = [
        Pedal::Comp,
        Pedal::Drive,
        Pedal::Phaser,
        Pedal::Chorus,
        Pedal::Flanger,
        Pedal::Tremolo,
        Pedal::Delay,
        Pedal::Reverb,
    ];

    fn in_front(order: &Order, pedal: Pedal) -> bool {
        order.pedals[..order.front].contains(&pedal)
    }

    #[test]
    fn the_row_fills_the_window() {
        let order = Order::default();
        let (lefts, gap) = order.places();
        let last = order.pedals[PEDALS - 1];
        assert_eq!(lefts[last as usize] + last.width() + BENCH_MARGIN, WINDOW_WIDTH as f32);
        assert_eq!(lefts[Pedal::Cab as usize], gap + AMP_GAP);
        assert_eq!(lefts[Pedal::Gate as usize], BENCH_MARGIN);
    }

    #[test]
    fn no_move_breaks_the_rules() {
        // Every order that is two moves away from the default one
        let mut orders = vec![Order::default()];
        for _ in 0..2 {
            orders = orders
                .iter()
                .flat_map(|order| MOVABLE.iter().flat_map(|&pedal| order.moves(pedal)))
                .collect();
        }
        assert!(orders.len() > 1000);
        for order in orders {
            assert_eq!(order.pedals[0], Pedal::Gate, "{order:?}");
            assert_eq!(order.pedals[order.front], Pedal::Cab, "{order:?}");
            assert!(in_front(&order, Pedal::Drive), "{order:?}");
            assert!(
                !in_front(&order, Pedal::Delay) && !in_front(&order, Pedal::Reverb),
                "{order:?}"
            );
            for pedal in MOVABLE {
                assert_eq!(
                    order.pedals.iter().filter(|&&other| other == pedal).count(),
                    1,
                    "{order:?}"
                );
            }
        }
    }

    #[test]
    fn the_row_is_the_chains_order_and_back() {
        // The default row is the default chain, and every row two moves from it comes
        // back from the chain's order as it was
        assert_eq!(Order::of_chain(&ChainOrder::default()), Order::default());
        let mut orders = vec![Order::default()];
        for _ in 0..2 {
            orders = orders
                .iter()
                .flat_map(|order| MOVABLE.iter().flat_map(|&pedal| order.moves(pedal)))
                .collect();
        }
        for order in orders {
            assert_eq!(Order::of_chain(&order.chain()), order, "{order:?}");
        }
        for pedal in MOVABLE {
            assert_eq!(pedal.in_chain().map(Pedal::of_chain), Some(pedal));
        }
    }

    #[test]
    fn fixed_pedals_have_no_moves() {
        assert!(Order::default().moves(Pedal::Gate).is_empty());
        assert!(Order::default().moves(Pedal::Cab).is_empty());
        assert_eq!(Order::default().nearest(Pedal::Cab, 0.0), Order::default());
    }

    #[test]
    fn a_pedal_lands_in_the_nearest_place_it_may_have() {
        use Pedal::*;
        let order = Order::default();
        // Let go where it stands, nothing moves
        for pedal in MOVABLE {
            assert_eq!(order.nearest(pedal, order.places().0[pedal as usize]), order);
        }
        // The chorus across the amp to right behind the gate
        let moved = order.nearest(Chorus, 0.0);
        assert_eq!(
            moved.pedals,
            [Gate, Chorus, Comp, Drive, Phaser, Cab, Flanger, Tremolo, Delay, Reverb]
        );
        assert_eq!(moved.front, 5);
        // The drive as far right as it goes: last in front of the amp
        let moved = order.nearest(Drive, WINDOW_WIDTH as f32);
        assert_eq!(
            moved.pedals,
            [Gate, Comp, Phaser, Drive, Cab, Chorus, Flanger, Tremolo, Delay, Reverb]
        );
        assert_eq!(moved.front, 4);
        // The reverb as far left as it goes: first behind the cabinet
        let moved = order.nearest(Reverb, 0.0);
        assert_eq!(
            moved.pedals,
            [Gate, Comp, Drive, Phaser, Cab, Reverb, Chorus, Flanger, Tremolo, Delay]
        );
        assert_eq!(moved.front, 4);
        // The phaser to the far end
        let moved = order.nearest(Phaser, WINDOW_WIDTH as f32);
        assert_eq!(
            moved.pedals,
            [Gate, Comp, Drive, Cab, Chorus, Flanger, Tremolo, Delay, Reverb, Phaser]
        );
        assert_eq!(moved.front, 3);
    }
}
