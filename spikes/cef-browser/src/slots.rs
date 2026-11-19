//! The slots the page reports and what the host does with each (S-16): one embedded browser per slot, drawn over it
//! while the slot is visible and nothing of omfp's own UI covers it. When something covers it (the `:` line, a dialog,
//! a toast), the browser freezes: the host captures it, the page shows the capture in the slot, then the host hides
//! the native view, so omfp's UI shows on top. Uncovered, the capture goes and the view shows again.

use std::collections::BTreeMap;

use serde::Deserialize;

/// A rectangle in CSS pixels of the main page, which are the window's DIPs at zoom 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// One `[data-omfp-cef-browser]` element of the page, as the bridge script reports it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Report {
    pub id: String,
    pub url: String,
    #[serde(flatten)]
    pub bounds: Bounds,
    /// Laid out with a size and not hidden by CSS (another view or tab).
    pub visible: bool,
    /// Another element of the page is on top of some part of it.
    pub covered: bool,
}

/// Whether the embedded browser is live, being captured, or replaced by its capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Live,
    Freezing,
    Frozen,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    pub url: String,
    pub bounds: Bounds,
    /// The native view is visible.
    pub shown: bool,
    pub phase: Phase,
}

/// What the host does to the native views and the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// A new embedded browser, hidden, at `bounds`.
    Create {
        id: String,
        url: String,
        bounds: Bounds,
    },
    Navigate {
        id: String,
        url: String,
    },
    Move {
        id: String,
        bounds: Bounds,
    },
    Show {
        id: String,
    },
    Hide {
        id: String,
    },
    /// Capture the browser; once the page shows the capture, [`Slots::frozen`] hides the view.
    Freeze {
        id: String,
    },
    /// The page drops the capture.
    Thaw {
        id: String,
    },
    Destroy {
        id: String,
    },
}

#[derive(Debug, Default)]
pub struct Slots {
    slots: BTreeMap<String, Slot>,
}

impl Slots {
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Slot> {
        self.slots.get(id)
    }

    /// The actions that bring the views in line with the page's latest report of every slot.
    pub fn update(&mut self, reports: &[Report]) -> Vec<Action> {
        let mut actions = Vec::new();
        let gone: Vec<String> = self
            .slots
            .keys()
            .filter(|id| !reports.iter().any(|report| &report.id == *id))
            .cloned()
            .collect();
        for id in gone {
            self.slots.remove(&id);
            actions.push(Action::Destroy { id });
        }
        for report in reports {
            self.apply(report, &mut actions);
        }
        actions
    }

    fn apply(&mut self, report: &Report, actions: &mut Vec<Action>) {
        let id = report.id.clone();
        let slot = self.slots.entry(id.clone()).or_insert_with(|| {
            actions.push(Action::Create {
                id: id.clone(),
                url: report.url.clone(),
                bounds: report.bounds,
            });
            Slot {
                url: report.url.clone(),
                bounds: report.bounds,
                shown: false,
                phase: Phase::Live,
            }
        });
        if slot.url != report.url {
            slot.url.clone_from(&report.url);
            actions.push(Action::Navigate {
                id: id.clone(),
                url: report.url.clone(),
            });
        }
        if slot.bounds != report.bounds {
            slot.bounds = report.bounds;
            actions.push(Action::Move {
                id: id.clone(),
                bounds: report.bounds,
            });
        }
        if slot.phase != Phase::Live && !report.covered {
            slot.phase = Phase::Live;
            actions.push(Action::Thaw { id: id.clone() });
        }
        if slot.phase == Phase::Live && report.visible && report.covered {
            slot.phase = Phase::Freezing;
            actions.push(Action::Freeze { id: id.clone() });
        }
        let show = report.visible && slot.phase != Phase::Frozen;
        if show != slot.shown {
            slot.shown = show;
            actions.push(if show {
                Action::Show { id }
            } else {
                Action::Hide { id }
            });
        }
    }

    /// The page shows the capture of `id`: hide its view, unless it was uncovered meanwhile (the capture then came too
    /// late and the host must not show it).
    pub fn frozen(&mut self, id: &str) -> Vec<Action> {
        let Some(slot) = self.slots.get_mut(id) else {
            return Vec::new();
        };
        if slot.phase != Phase::Freezing {
            return Vec::new();
        }
        slot.phase = Phase::Frozen;
        if slot.shown {
            slot.shown = false;
            return vec![Action::Hide { id: id.to_owned() }];
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: Bounds = Bounds {
        x: 10,
        y: 20,
        w: 300,
        h: 200,
    };

    fn report(visible: bool, covered: bool) -> Report {
        Report {
            id: "yt".to_owned(),
            url: "https://www.youtube.com/".to_owned(),
            bounds: AT,
            visible,
            covered,
        }
    }

    fn id() -> String {
        "yt".to_owned()
    }

    #[test]
    fn a_new_visible_slot_gets_a_browser_shown_at_its_place() {
        let mut slots = Slots::default();
        assert_eq!(
            slots.update(&[report(true, false)]),
            vec![
                Action::Create {
                    id: id(),
                    url: "https://www.youtube.com/".to_owned(),
                    bounds: AT,
                },
                Action::Show { id: id() },
            ]
        );
        assert_eq!(slots.update(&[report(true, false)]), Vec::new());
    }

    #[test]
    fn a_hidden_slot_keeps_its_browser_hidden_until_it_shows() {
        let mut slots = Slots::default();
        slots.update(&[report(false, false)]);
        assert_eq!(slots.get("yt").map(|slot| slot.shown), Some(false));
        assert_eq!(
            slots.update(&[report(true, false)]),
            vec![Action::Show { id: id() }]
        );
        assert_eq!(
            slots.update(&[report(false, false)]),
            vec![Action::Hide { id: id() }]
        );
    }

    #[test]
    fn a_covered_slot_hides_its_view_only_once_the_page_shows_the_capture() {
        let mut slots = Slots::default();
        slots.update(&[report(true, false)]);
        assert_eq!(
            slots.update(&[report(true, true)]),
            vec![Action::Freeze { id: id() }]
        );
        assert_eq!(slots.get("yt").map(|slot| slot.shown), Some(true));
        assert_eq!(slots.frozen("yt"), vec![Action::Hide { id: id() }]);
        assert_eq!(slots.update(&[report(true, true)]), Vec::new());
    }

    #[test]
    fn uncovered_the_capture_goes_and_the_view_shows_again() {
        let mut slots = Slots::default();
        slots.update(&[report(true, false)]);
        slots.update(&[report(true, true)]);
        slots.frozen("yt");
        assert_eq!(
            slots.update(&[report(true, false)]),
            vec![Action::Thaw { id: id() }, Action::Show { id: id() }]
        );
    }

    #[test]
    fn a_capture_arriving_after_the_slot_was_uncovered_changes_nothing() {
        let mut slots = Slots::default();
        slots.update(&[report(true, false)]);
        slots.update(&[report(true, true)]);
        assert_eq!(
            slots.update(&[report(true, false)]),
            vec![Action::Thaw { id: id() }]
        );
        assert_eq!(slots.frozen("yt"), Vec::new());
        assert_eq!(
            slots.get("yt").map(|slot| (slot.shown, slot.phase)),
            Some((true, Phase::Live))
        );
    }

    #[test]
    fn moved_or_renavigated_slots_move_or_navigate_their_browser() {
        let mut slots = Slots::default();
        slots.update(&[report(true, false)]);
        let moved = Report {
            url: "https://example.com/".to_owned(),
            bounds: Bounds { x: 0, ..AT },
            ..report(true, false)
        };
        assert_eq!(
            slots.update(&[moved]),
            vec![
                Action::Navigate {
                    id: id(),
                    url: "https://example.com/".to_owned(),
                },
                Action::Move {
                    id: id(),
                    bounds: Bounds { x: 0, ..AT },
                },
            ]
        );
    }

    #[test]
    fn a_slot_gone_from_the_page_destroys_its_browser() {
        let mut slots = Slots::default();
        slots.update(&[report(true, false)]);
        assert_eq!(slots.update(&[]), vec![Action::Destroy { id: id() }]);
        assert_eq!(slots.get("yt"), None);
    }

    #[test]
    fn a_frozen_slot_switched_away_and_back_uncovered_shows_live() {
        let mut slots = Slots::default();
        slots.update(&[report(true, false)]);
        slots.update(&[report(true, true)]);
        slots.frozen("yt");
        assert_eq!(
            slots.update(&[report(false, false)]),
            vec![Action::Thaw { id: id() }]
        );
        assert_eq!(
            slots.update(&[report(true, false)]),
            vec![Action::Show { id: id() }]
        );
    }
}
