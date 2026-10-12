//! The cells the dashboard asks, and what the page shows of each.
//!
//! Two facts about the runtime shape this (docs/myrmic-notes.md, section 16):
//!
//! - A command to a cell that is not deployed is kept and delivered when the
//!   cell comes back. Asking on every refresh regardless would queue one
//!   query per refresh in front of a returning cell's readings, so a cell is
//!   asked again only once it has answered, or after a long silence in case
//!   the query was lost.
//! - The blob store the page's files live in never deletes content: every
//!   file written stays on the node. An answer that says what the page
//!   already shows is therefore not written again ([`fingerprint`]).

use myrmic_sdk::Sri;

/// How many refreshes a cell may leave a query unanswered before its last
/// answer is withdrawn from the page. An answer that has stopped being
/// renewed must not go on looking current.
pub(crate) const SILENT_AFTER: u32 = 3;

/// How many refreshes pass between two queries to a cell that does not
/// answer.
pub(crate) const ASK_AGAIN_AFTER: u32 = 30;

/// What a refresh does about one cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refresh {
    /// Nothing is outstanding: ask.
    Ask,
    /// A query is outstanding: wait for its answer.
    Wait,
    /// The query has been outstanding for [`SILENT_AFTER`] refreshes:
    /// withdraw the cell's last answer from the page, and keep waiting.
    Withdraw,
    /// The query has been outstanding for a multiple of [`ASK_AGAIN_AFTER`]
    /// refreshes: ask again.
    AskAgain,
}

/// One cell the dashboard asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Source {
    /// The cell, as the runtime addresses it.
    pub cell: Sri,
    /// How many refreshes the outstanding query has waited, if one is
    /// outstanding.
    waited: Option<u32>,
    /// The [`fingerprint`] of the answer the page shows, if it shows one.
    shown: Option<u64>,
}

impl Source {
    /// A cell that has not been asked yet.
    pub(crate) const fn new(cell: Sri) -> Self {
        Self {
            cell,
            waited: None,
            shown: None,
        }
    }

    /// Counts one refresh and says what to do about this cell.
    pub(crate) fn refresh(&mut self) -> Refresh {
        let Some(waited) = self.waited else {
            self.waited = Some(0);
            return Refresh::Ask;
        };
        let waited = waited.saturating_add(1);
        self.waited = Some(waited);
        if waited == SILENT_AFTER {
            self.shown = None;
            Refresh::Withdraw
        } else if waited.is_multiple_of(ASK_AGAIN_AFTER) {
            Refresh::AskAgain
        } else {
            Refresh::Wait
        }
    }

    /// Forgets the outstanding query, for one that could not be sent: the
    /// next refresh asks.
    pub(crate) const fn not_asked(&mut self) {
        self.waited = None;
    }

    /// Records that the cell answered. Returns whether its answers had been
    /// withdrawn from the page.
    pub(crate) fn answered(&mut self) -> bool {
        let withdrawn = self.waited.is_some_and(|waited| waited >= SILENT_AFTER);
        self.waited = None;
        withdrawn
    }

    /// Records the answer the page is about to show. Returns `false` when
    /// the page shows that answer already and nothing needs writing.
    pub(crate) fn show(&mut self, fingerprint: u64) -> bool {
        let changed = self.shown != Some(fingerprint);
        self.shown = Some(fingerprint);
        changed
    }

    /// Records that the answer could not be written, so the next one is.
    pub(crate) const fn not_shown(&mut self) {
        self.shown = None;
    }
}

/// The two cells the dashboard asks about one refrigerator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Route {
    /// The asset cell: asked for the status snapshot.
    pub asset: Source,
    /// The risk monitor cell: asked for the incidents.
    pub monitor: Source,
}

/// A 64-bit FNV-1a hash: enough to tell whether an answer says what the last
/// one said without keeping the last one.
pub(crate) fn fingerprint(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    bytes.iter().fold(OFFSET_BASIS, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(PRIME)
    })
}

#[cfg(test)]
mod tests {
    use myrmic_sdk::Vec;

    use super::*;

    fn source() -> Source {
        Source::new(Sri::of_path("fridge-asset-cell").unwrap())
    }

    #[test]
    fn a_cell_that_answers_is_asked_on_every_refresh() {
        let mut source = source();
        for _ in 0..3 {
            assert_eq!(source.refresh(), Refresh::Ask);
            assert!(!source.answered());
        }
    }

    #[test]
    fn a_silent_cell_is_asked_once_withdrawn_once_and_asked_again_rarely() {
        let mut source = source();
        let refreshes: Vec<Refresh> = (0..=2 * ASK_AGAIN_AFTER)
            .map(|_| source.refresh())
            .collect();
        let at = |what| -> Vec<usize> {
            refreshes
                .iter()
                .enumerate()
                .filter(|(_, refresh)| **refresh == what)
                .map(|(n, _)| n)
                .collect()
        };
        let silent = usize::try_from(SILENT_AFTER).unwrap();
        let again = usize::try_from(ASK_AGAIN_AFTER).unwrap();
        assert_eq!(at(Refresh::Ask), [0]);
        assert_eq!(at(Refresh::Withdraw), [silent]);
        assert_eq!(at(Refresh::AskAgain), [again, 2 * again]);
    }

    #[test]
    fn an_answer_resumes_asking_and_says_whether_the_page_had_lost_it() {
        let mut source = source();
        assert_eq!(source.refresh(), Refresh::Ask);
        assert_eq!(source.refresh(), Refresh::Wait);
        assert!(!source.answered(), "late, but not withdrawn yet");
        assert_eq!(source.refresh(), Refresh::Ask);
        for _ in 0..SILENT_AFTER {
            source.refresh();
        }
        assert!(source.answered(), "withdrawn after SILENT_AFTER refreshes");
        // An answer nobody is waiting for (a query the runtime had kept for
        // a cell that was away) changes nothing.
        assert!(!source.answered());
        assert_eq!(source.refresh(), Refresh::Ask);
    }

    #[test]
    fn a_query_that_was_not_sent_is_asked_on_the_next_refresh() {
        let mut source = source();
        assert_eq!(source.refresh(), Refresh::Ask);
        source.not_asked();
        assert_eq!(source.refresh(), Refresh::Ask);
    }

    #[test]
    fn an_answer_the_page_shows_is_not_written_again() {
        let mut source = source();
        let first = fingerprint(b"one incident");
        assert!(source.show(first));
        assert!(!source.show(first));
        assert!(source.show(fingerprint(b"two incidents")));
        assert!(source.show(first));
        source.not_shown();
        assert!(source.show(first), "a failed write is repeated");
    }

    #[test]
    fn a_withdrawn_answer_is_written_again_when_the_cell_returns() {
        let mut source = source();
        let same = fingerprint(b"no incidents");
        assert_eq!(source.refresh(), Refresh::Ask);
        assert!(source.show(same));
        for _ in 0..SILENT_AFTER {
            source.refresh();
        }
        assert!(source.answered());
        assert!(source.show(same), "the file was removed, so it is due");
    }

    #[test]
    fn fingerprint_is_fnv_1a_64() {
        // Reference vectors of the FNV-1a specification.
        assert_eq!(fingerprint(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fingerprint(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fingerprint(b"foobar"), 0x8594_4171_f739_67e8);
    }
}
