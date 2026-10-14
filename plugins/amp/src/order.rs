//! The order the pedals stand in: which of them are in front of the amp and which behind
//! the cabinet, and in what order within each of the two.
//!
//! Not every pedal can stand everywhere. The gate is always first, since it listens to the
//! guitar as it arrives, and the amp and its cabinet are one block in the middle: neither is
//! part of the order. The drive pedal stays in front of the amp, where it shares the amp's
//! oversampling. The delay and the reverb stay behind the cabinet, where the signal is
//! stereo. The compressor and the modulation pedals go either side.
//!
//! The order is not a parameter: a host cannot automate an arrangement. The editor changes
//! it, it is saved with the project, and the chain reads it with the dials.

use nih_plug::params::persist::PersistentField;
use std::sync::atomic::{AtomicU64, Ordering};

/// The pedals that can be moved
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pedal {
    Comp,
    Drive,
    Chorus,
    Phaser,
    Flanger,
    Tremolo,
    Delay,
    Reverb,
}

/// The two stretches of the chain a pedal can stand in
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    /// Between the gate and the amp: one channel, at the amp's oversampled rate
    Front,
    /// Behind the cabinet: two channels, at the host's rate
    Back,
}

impl Pedal {
    pub const ALL: [Pedal; 8] = [
        Pedal::Comp,
        Pedal::Drive,
        Pedal::Chorus,
        Pedal::Phaser,
        Pedal::Flanger,
        Pedal::Tremolo,
        Pedal::Delay,
        Pedal::Reverb,
    ];

    /// Frozen: what a saved project calls the pedal
    pub fn id(self) -> &'static str {
        match self {
            Pedal::Comp => "comp",
            Pedal::Drive => "drive",
            Pedal::Chorus => "chorus",
            Pedal::Phaser => "phaser",
            Pedal::Flanger => "flanger",
            Pedal::Tremolo => "tremolo",
            Pedal::Delay => "delay",
            Pedal::Reverb => "reverb",
        }
    }

    fn from_id(id: &str) -> Option<Pedal> {
        Pedal::ALL.into_iter().find(|pedal| pedal.id() == id)
    }

    /// Position in `ALL`
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn can_be_in(self, zone: Zone) -> bool {
        match self {
            Pedal::Drive => zone == Zone::Front,
            Pedal::Delay | Pedal::Reverb => zone == Zone::Back,
            Pedal::Comp | Pedal::Chorus | Pedal::Phaser | Pedal::Flanger | Pedal::Tremolo => true,
        }
    }
}

/// What a saved project calls the amp and its cabinet, between the two zones
pub const AMP_ID: &str = "amp";

/// Every pedal once: the first `front` of them in front of the amp in that order, the rest
/// behind the cabinet in theirs. Always a legal arrangement
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainOrder {
    pedals: [Pedal; Pedal::ALL.len()],
    front: usize,
}

/// The chain as it was before it could be rearranged, with the pedals that came later
/// where they are used most: the compressor first and the phaser in front of the amp, the
/// other modulation behind the cabinet in front of the delay
impl Default for ChainOrder {
    fn default() -> Self {
        Self {
            pedals: [
                Pedal::Comp,
                Pedal::Drive,
                Pedal::Phaser,
                Pedal::Chorus,
                Pedal::Flanger,
                Pedal::Tremolo,
                Pedal::Delay,
                Pedal::Reverb,
            ],
            front: 3,
        }
    }
}

impl ChainOrder {
    /// In front of the amp, first to last
    pub fn front(&self) -> &[Pedal] {
        &self.pedals[..self.front]
    }

    /// Behind the cabinet, first to last
    pub fn back(&self) -> &[Pedal] {
        &self.pedals[self.front..]
    }

    pub fn zone_of(&self, pedal: Pedal) -> Zone {
        if self.front().contains(&pedal) {
            Zone::Front
        } else {
            Zone::Back
        }
    }

    /// The order with `pedal` taken out and put back as number `place` of `zone` (0 is
    /// first; past the end is last). A pedal that cannot stand in that zone stays where it is
    // For the tests of the chain: the board works out a whole row and hands it over as ids
    #[cfg(test)]
    pub fn moved(&self, pedal: Pedal, zone: Zone, place: usize) -> ChainOrder {
        if !pedal.can_be_in(zone) {
            return *self;
        }
        let mut rows = [Row::new(), Row::new()];
        for (row, pedals) in rows.iter_mut().zip([self.front(), self.back()]) {
            for &other in pedals.iter().filter(|&&other| other != pedal) {
                row.push(other);
            }
        }
        let row = &mut rows[zone as usize];
        row.insert(place.min(row.len), pedal);
        Self::from_rows(&rows)
    }

    /// From two rows that hold every pedal once between them
    fn from_rows(rows: &[Row; 2]) -> ChainOrder {
        let [front, back] = rows;
        debug_assert_eq!(front.len + back.len, Pedal::ALL.len());
        let mut pedals = [Pedal::Comp; Pedal::ALL.len()];
        for (slot, &pedal) in pedals.iter_mut().zip(front.pedals().iter().chain(back.pedals())) {
            *slot = pedal;
        }
        ChainOrder { pedals, front: front.len }
    }

    /// The order a saved project names: the ids of the pedals with `amp` between the two
    /// zones. Whatever is wrong with the list is put right and not refused, since a project
    /// has to open: a name this build does not know is skipped, a pedal named twice counts
    /// once, a pedal on the side it cannot stand on and one that is not named at all go to
    /// their zone of the default order, behind the pedals that are in front of them there
    pub fn from_ids<'a>(ids: impl IntoIterator<Item = &'a str>) -> ChainOrder {
        let default = ChainOrder::default();
        let mut rows = [Row::new(), Row::new()];
        let mut zone = Zone::Front;
        let mut named = [false; Pedal::ALL.len()];
        for id in ids {
            if id == AMP_ID {
                zone = Zone::Back;
            } else if let Some(pedal) = Pedal::from_id(id) {
                if std::mem::replace(&mut named[pedal.index()], true) {
                    continue;
                }
                if pedal.can_be_in(zone) {
                    rows[zone as usize].push(pedal);
                }
            }
        }

        // The pedals without a place, in the order of the default: each goes behind the
        // last of those that the default has in front of it in its zone
        for pedal in default.pedals {
            if rows.iter().any(|row| row.pedals().contains(&pedal)) {
                continue;
            }
            let zone = default.zone_of(pedal);
            let ahead = if zone == Zone::Front { default.front() } else { default.back() };
            let ahead = &ahead[..ahead.iter().position(|&other| other == pedal).unwrap_or(0)];
            let row = &mut rows[zone as usize];
            let place = row
                .pedals()
                .iter()
                .rposition(|other| ahead.contains(other))
                .map_or(0, |last| last + 1);
            row.insert(place, pedal);
        }
        Self::from_rows(&rows)
    }

    /// What a saved project names the order as: see `from_ids`
    pub fn ids(&self) -> Vec<&'static str> {
        let front = self.front().iter().map(|pedal| pedal.id());
        let back = self.back().iter().map(|pedal| pedal.id());
        front.chain(std::iter::once(AMP_ID)).chain(back).collect()
    }

    /// The order as one number: four bits per pedal, first to last, then how many of them
    /// are in front of the amp
    fn to_bits(self) -> u64 {
        let pedals =
            self.pedals.iter().rev().fold(0u64, |bits, pedal| (bits << 4) | pedal.index() as u64);
        pedals | ((self.front as u64) << (4 * Pedal::ALL.len()))
    }

    /// The order a number from `to_bits` stands for. `None` for any other number
    fn from_bits(bits: u64) -> Option<ChainOrder> {
        let mut pedals = [Pedal::Comp; Pedal::ALL.len()];
        let mut seen = [false; Pedal::ALL.len()];
        for (place, slot) in pedals.iter_mut().enumerate() {
            let index = ((bits >> (4 * place)) & 0xf) as usize;
            let pedal = *Pedal::ALL.get(index)?;
            if std::mem::replace(&mut seen[index], true) {
                return None;
            }
            *slot = pedal;
        }
        let front = (bits >> (4 * Pedal::ALL.len())) as usize;
        let order = ChainOrder { pedals, front };
        let legal = front <= Pedal::ALL.len()
            && order.front().iter().all(|pedal| pedal.can_be_in(Zone::Front))
            && order.back().iter().all(|pedal| pedal.can_be_in(Zone::Back));
        legal.then_some(order)
    }
}

/// One of the two rows while an order is put together. Holds no memory of its own: an order
/// may be restored from a saved project on the audio thread
#[derive(Clone, Copy)]
struct Row {
    slots: [Pedal; Pedal::ALL.len()],
    len: usize,
}

impl Row {
    fn new() -> Self {
        Self { slots: [Pedal::Comp; Pedal::ALL.len()], len: 0 }
    }

    fn pedals(&self) -> &[Pedal] {
        &self.slots[..self.len]
    }

    fn push(&mut self, pedal: Pedal) {
        self.insert(self.len, pedal);
    }

    /// Does nothing once the row holds every pedal there is
    fn insert(&mut self, place: usize, pedal: Pedal) {
        if self.len < self.slots.len() {
            self.slots.copy_within(place..self.len, place + 1);
            self.slots[place] = pedal;
            self.len += 1;
        }
    }
}

/// The order as the editor sets it and the audio thread reads it, without either waiting
/// for the other. A host may restore a saved state from the audio thread, so it is one atomic
pub struct StoredOrder {
    bits: AtomicU64,
}

impl StoredOrder {
    pub fn new() -> Self {
        Self { bits: AtomicU64::new(ChainOrder::default().to_bits()) }
    }

    pub fn get(&self) -> ChainOrder {
        ChainOrder::from_bits(self.bits.load(Ordering::Relaxed)).unwrap_or_default()
    }

    pub fn put(&self, order: ChainOrder) {
        self.bits.store(order.to_bits(), Ordering::Relaxed);
    }
}

/// Saved with the project as the list of `ChainOrder::ids`. A project from before the
/// pedals could be moved has no list and keeps the default
impl PersistentField<'_, Vec<String>> for StoredOrder {
    fn set(&self, new_value: Vec<String>) {
        self.put(ChainOrder::from_ids(new_value.iter().map(String::as_str)));
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&Vec<String>) -> R,
    {
        f(&self.get().ids().into_iter().map(String::from).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_legal(order: &ChainOrder) -> bool {
        let every_pedal_once = Pedal::ALL
            .iter()
            .all(|pedal| order.pedals.iter().filter(|other| *other == pedal).count() == 1);
        every_pedal_once
            && order.front().iter().all(|pedal| pedal.can_be_in(Zone::Front))
            && order.back().iter().all(|pedal| pedal.can_be_in(Zone::Back))
    }

    #[test]
    fn test_default_is_the_chain_as_it_was_with_the_new_pedals_around_it() {
        let order = ChainOrder::default();
        assert!(is_legal(&order));
        assert_eq!(order.front(), [Pedal::Comp, Pedal::Drive, Pedal::Phaser]);
        assert_eq!(
            order.back(),
            [Pedal::Chorus, Pedal::Flanger, Pedal::Tremolo, Pedal::Delay, Pedal::Reverb]
        );
        // The pedals the chain had before keep their order: drive, amp, delay, reverb
        let before = |first: Pedal, second: Pedal| {
            let place = |pedal| order.pedals.iter().position(|&other| other == pedal).unwrap();
            place(first) < place(second)
        };
        assert!(order.zone_of(Pedal::Drive) == Zone::Front && before(Pedal::Delay, Pedal::Reverb));
        assert_eq!(
            order.ids(),
            ["comp", "drive", "phaser", "amp", "chorus", "flanger", "tremolo", "delay", "reverb"]
        );
    }

    #[test]
    fn test_ids_are_unique_and_known() {
        for pedal in Pedal::ALL {
            assert_eq!(Pedal::from_id(pedal.id()), Some(pedal));
            assert_eq!(Pedal::ALL[pedal.index()], pedal);
            assert_ne!(pedal.id(), AMP_ID);
        }
        assert_eq!(Pedal::from_id("wah"), None);
    }

    #[test]
    fn test_a_pedal_moves_within_its_zone_and_across_the_amp() {
        let order = ChainOrder::default();

        let chorus_first = order.moved(Pedal::Chorus, Zone::Front, 0);
        assert_eq!(chorus_first.front(), [Pedal::Chorus, Pedal::Comp, Pedal::Drive, Pedal::Phaser]);
        assert_eq!(
            chorus_first.back(),
            [Pedal::Flanger, Pedal::Tremolo, Pedal::Delay, Pedal::Reverb]
        );

        let reverb_into_delay = order.moved(Pedal::Reverb, Zone::Back, 3);
        assert_eq!(
            reverb_into_delay.back(),
            [Pedal::Chorus, Pedal::Flanger, Pedal::Tremolo, Pedal::Reverb, Pedal::Delay]
        );

        let comp_behind_drive = order.moved(Pedal::Comp, Zone::Front, 1);
        assert_eq!(comp_behind_drive.front(), [Pedal::Drive, Pedal::Comp, Pedal::Phaser]);

        // Past the end is last, and where a pedal already stands is no move
        assert_eq!(order.moved(Pedal::Comp, Zone::Back, 99).back().last(), Some(&Pedal::Comp));
        assert_eq!(order.moved(Pedal::Drive, Zone::Front, 1), order);
        for moved in [chorus_first, reverb_into_delay, comp_behind_drive] {
            assert!(is_legal(&moved));
        }
    }

    #[test]
    fn test_no_move_takes_a_pedal_where_it_cannot_stand() {
        let order = ChainOrder::default();
        assert_eq!(order.moved(Pedal::Drive, Zone::Back, 0), order);
        assert_eq!(order.moved(Pedal::Delay, Zone::Front, 0), order);
        assert_eq!(order.moved(Pedal::Reverb, Zone::Front, 2), order);

        // Every move from every order that a few moves lead to
        let mut orders = vec![order];
        for _ in 0..2 {
            for from in orders.clone() {
                for pedal in Pedal::ALL {
                    for zone in [Zone::Front, Zone::Back] {
                        for place in 0..=Pedal::ALL.len() {
                            let moved = from.moved(pedal, zone, place);
                            assert!(is_legal(&moved), "{from:?}: {pedal:?} to {zone:?} {place}");
                            if !orders.contains(&moved) {
                                orders.push(moved);
                            }
                        }
                    }
                }
            }
        }
        assert!(orders.len() > 100);

        // And each of them comes back from its number and from its names as it was
        for order in orders {
            assert_eq!(ChainOrder::from_bits(order.to_bits()), Some(order));
            assert_eq!(ChainOrder::from_ids(order.ids()), order);
        }
    }

    #[test]
    fn test_a_damaged_list_still_gives_a_legal_order() {
        let default = ChainOrder::default();
        // No list at all, and a list from a build with fewer pedals: the pedals of before
        // where the project had them, the others where the default has them
        assert_eq!(ChainOrder::from_ids([]), default);
        let older = ChainOrder::from_ids(["drive", "amp", "reverb", "delay"]);
        assert_eq!(older.front(), [Pedal::Comp, Pedal::Drive, Pedal::Phaser]);
        assert_eq!(
            older.back(),
            [Pedal::Chorus, Pedal::Flanger, Pedal::Tremolo, Pedal::Reverb, Pedal::Delay]
        );

        // Names of a later build, a pedal twice, pedals on the wrong side, no amp
        let odd = ChainOrder::from_ids([
            "wah", "chorus", "chorus", "delay", "amp", "drive", "comp", "octaver",
        ]);
        assert!(is_legal(&odd));
        assert_eq!(odd.zone_of(Pedal::Chorus), Zone::Front);
        assert_eq!(odd.zone_of(Pedal::Comp), Zone::Back);
        assert_eq!(odd.zone_of(Pedal::Drive), Zone::Front);
        assert_eq!(odd.zone_of(Pedal::Delay), Zone::Back);
        let all_in_front = ChainOrder::from_ids(Pedal::ALL.map(Pedal::id));
        assert!(is_legal(&all_in_front));
        assert_eq!(all_in_front.back(), [Pedal::Delay, Pedal::Reverb]);

        for bits in [0, u64::MAX, default.to_bits() ^ 1, default.to_bits() | (0xf << 32)] {
            assert_eq!(ChainOrder::from_bits(bits), None, "{bits:#x}");
        }
    }

    #[test]
    fn test_stored_order_starts_at_the_default_and_survives_saving() {
        let stored = StoredOrder::new();
        assert_eq!(stored.get(), ChainOrder::default());

        let order = ChainOrder::default().moved(Pedal::Tremolo, Zone::Front, 0).moved(
            Pedal::Reverb,
            Zone::Back,
            0,
        );
        stored.put(order);
        assert_eq!(stored.get(), order);
        let saved = stored.map(|ids| ids.clone());
        assert_eq!(saved[0], "tremolo");

        let restored = StoredOrder::new();
        restored.set(saved);
        assert_eq!(restored.get(), order);

        // A number no order has, from a damaged state, reads as the default
        stored.bits.store(0, Ordering::Relaxed);
        assert_eq!(stored.get(), ChainOrder::default());
    }
}
