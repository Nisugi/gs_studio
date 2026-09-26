//! Closing the window with unsaved work in more than one tool (the rig
//! editor, the stage scene). Each tool that guards work is asked in turn;
//! the close waits while one has its question up, goes through once every
//! one agrees, and a Cancel anywhere calls the whole close off -- including
//! an agreement an earlier tool already gave, so the next close asks again
//! from the start instead of re-asking the cancelled question every frame.
//!
//! No egui here: the app feeds it the window's close request and does what
//! [`CloseSequence::step`] says.

/// A tool with work a window close must not silently lose.
pub trait QuitGuard {
    /// Asked once per close: true when nothing stands in the way, else the
    /// tool now has its question up.
    fn approve_quit(&mut self) -> bool;
    /// Its question is up, waiting on an answer.
    fn quit_asking(&self) -> bool;
    /// It has agreed to the close.
    fn quit_approved(&self) -> bool;
    /// The close is off: forget any agreement and any open question.
    fn cancel_quit(&mut self);
}

/// What the app does after a [`CloseSequence::step`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseStep {
    /// No close under way.
    Idle,
    /// A question is up; keep drawing.
    Waiting,
    /// Every guard agreed: close the window.
    Close,
    /// A guard's question was cancelled: the window stays open.
    Cancelled,
}

#[derive(Debug, Default)]
pub struct CloseSequence {
    active: bool,
    /// Guards asked so far, in order.
    asked: usize,
}

impl CloseSequence {
    /// The window asked to close. True when every guard already agrees (let
    /// it); otherwise the close is held and the guards will be asked.
    pub fn close_requested(&mut self, guards: &mut [&mut dyn QuitGuard]) -> bool {
        if guards.iter().all(|g| g.quit_approved()) {
            self.active = false;
            return true;
        }
        if !self.active {
            self.active = true;
            self.asked = 0;
        }
        false
    }

    /// One frame of a close under way: ask the next guard, wait on an open
    /// question, or settle it.
    pub fn step(&mut self, guards: &mut [&mut dyn QuitGuard]) -> CloseStep {
        if !self.active {
            return CloseStep::Idle;
        }
        let mut cancelled = false;
        for (i, guard) in guards.iter_mut().enumerate() {
            if guard.quit_approved() {
                continue;
            }
            if guard.quit_asking() {
                return CloseStep::Waiting;
            }
            if i < self.asked {
                // Asked, and neither asking nor agreeing: cancelled.
                cancelled = true;
                break;
            }
            self.asked = i + 1;
            if !guard.approve_quit() {
                return CloseStep::Waiting;
            }
        }
        self.active = false;
        if cancelled {
            for guard in guards.iter_mut() {
                guard.cancel_quit();
            }
            CloseStep::Cancelled
        } else {
            CloseStep::Close
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A guard with or without unsaved work; `answer` plays the user.
    #[derive(Default)]
    struct Fake {
        dirty: bool,
        asking: bool,
        approved: bool,
        asked: usize,
        cancels: usize,
    }

    impl Fake {
        fn dirty() -> Fake {
            Fake { dirty: true, ..Fake::default() }
        }
        fn answer(&mut self, discard: bool) {
            self.asking = false;
            self.approved = discard;
        }
    }

    impl QuitGuard for Fake {
        fn approve_quit(&mut self) -> bool {
            self.asked += 1;
            if self.dirty {
                self.asking = true;
            } else {
                self.approved = true;
            }
            self.approved
        }
        fn quit_asking(&self) -> bool {
            self.asking
        }
        fn quit_approved(&self) -> bool {
            self.approved
        }
        fn cancel_quit(&mut self) {
            self.cancels += 1;
            self.asking = false;
            self.approved = false;
        }
    }

    #[test]
    fn nothing_unsaved_closes_on_the_first_frame() {
        let (mut rig, mut scene) = (Fake::default(), Fake::default());
        let mut seq = CloseSequence::default();
        assert_eq!(seq.step(&mut [&mut rig, &mut scene]), CloseStep::Idle);
        assert!(!seq.close_requested(&mut [&mut rig, &mut scene]));
        assert_eq!(seq.step(&mut [&mut rig, &mut scene]), CloseStep::Close);
        assert!(seq.close_requested(&mut [&mut rig, &mut scene]), "the close sent after Close goes through");
        assert_eq!((rig.asked, scene.asked), (1, 1));
    }

    #[test]
    fn guards_are_asked_one_at_a_time_in_order() {
        let (mut rig, mut scene) = (Fake::dirty(), Fake::dirty());
        let mut seq = CloseSequence::default();
        assert!(!seq.close_requested(&mut [&mut rig, &mut scene]));
        assert_eq!(seq.step(&mut [&mut rig, &mut scene]), CloseStep::Waiting);
        assert_eq!(seq.step(&mut [&mut rig, &mut scene]), CloseStep::Waiting);
        assert_eq!((rig.asked, scene.asked), (1, 0), "the scene waits for the rig's answer");
        rig.answer(true);
        assert_eq!(seq.step(&mut [&mut rig, &mut scene]), CloseStep::Waiting);
        assert_eq!(scene.asked, 1);
        scene.answer(true);
        assert_eq!(seq.step(&mut [&mut rig, &mut scene]), CloseStep::Close);
    }

    /// The rig said Discard, the scene said Cancel: the window stays open,
    /// and the rig's agreement is dropped so the scene is not asked again
    /// every frame (and the next close asks the rig again too).
    #[test]
    fn a_cancel_anywhere_calls_the_whole_close_off() {
        let (mut rig, mut scene) = (Fake::dirty(), Fake::dirty());
        let mut seq = CloseSequence::default();
        seq.close_requested(&mut [&mut rig, &mut scene]);
        seq.step(&mut [&mut rig, &mut scene]);
        rig.answer(true);
        seq.step(&mut [&mut rig, &mut scene]);
        scene.answer(false);
        assert_eq!(seq.step(&mut [&mut rig, &mut scene]), CloseStep::Cancelled);
        assert!(!rig.approved && rig.cancels == 1 && scene.cancels == 1);
        assert_eq!(seq.step(&mut [&mut rig, &mut scene]), CloseStep::Idle);

        // The next close starts over from the first guard.
        assert!(!seq.close_requested(&mut [&mut rig, &mut scene]));
        seq.step(&mut [&mut rig, &mut scene]);
        assert_eq!((rig.asked, scene.asked), (2, 1));
    }

    #[test]
    fn a_second_close_request_while_asking_changes_nothing() {
        let (mut rig, mut scene) = (Fake::dirty(), Fake::default());
        let mut seq = CloseSequence::default();
        seq.close_requested(&mut [&mut rig, &mut scene]);
        seq.step(&mut [&mut rig, &mut scene]);
        assert!(!seq.close_requested(&mut [&mut rig, &mut scene]));
        assert_eq!(seq.step(&mut [&mut rig, &mut scene]), CloseStep::Waiting);
        assert_eq!(rig.asked, 1, "not asked twice");
    }
}
