//! The import state machine: declared states, legal transitions, terminal
//! protection. Pure logic; persistence guards live with the repository.

use ratatoskr_chatgpt_archive::receipt::state::{ImportState, TransitionError};

#[test]
fn happy_path_advances_stage_by_stage_and_finishes_on_terminal() {
    let mut run = ImportState::Received;
    for stage in [
        ImportState::Hashed,
        ImportState::Stored,
        ImportState::Inspected,
        ImportState::Parsed,
        ImportState::Reconciled,
    ] {
        run = run.advance(stage).expect("forward progress must be legal");
    }
    assert_eq!(run, ImportState::Reconciled);
    run = run
        .advance(ImportState::Completed)
        .expect("reconciled completes");
    assert_eq!(run, ImportState::Completed);
}

#[test]
fn reconciled_may_also_settle_as_partial() {
    let mut run = ImportState::Received;
    for stage in [
        ImportState::Hashed,
        ImportState::Stored,
        ImportState::Inspected,
        ImportState::Parsed,
        ImportState::Reconciled,
        ImportState::Partial,
    ] {
        run = run.advance(stage).expect("the chain to partial is legal");
    }
    assert_eq!(run, ImportState::Partial);
}

#[test]
fn skipping_a_stage_is_refused() {
    let error = ImportState::Received
        .advance(ImportState::Stored)
        .expect_err("received cannot jump straight to stored");
    assert!(matches!(error, TransitionError::IllegalJump { .. }));
    // And so cannot parsed, which sits two further along the chain.
    let error = ImportState::Hashed
        .advance(ImportState::Parsed)
        .expect_err("hashed cannot jump to parsed");
    assert!(matches!(error, TransitionError::IllegalJump { .. }));
}

#[test]
fn failed_is_reachable_from_every_non_terminal_state() {
    for stage in [
        ImportState::Received,
        ImportState::Hashed,
        ImportState::Stored,
        ImportState::Inspected,
        ImportState::Parsed,
        ImportState::Reconciled,
    ] {
        let failed = stage
            .clone()
            .advance(ImportState::Failed)
            .expect("any non-terminal stage may fail");
        assert_eq!(failed, ImportState::Failed, "from {stage:?}");
    }
}

#[test]
fn terminal_states_accept_no_transition() {
    for terminal in [
        ImportState::Completed,
        ImportState::Partial,
        ImportState::Failed,
        ImportState::Duplicate,
        ImportState::Quarantined,
    ] {
        let error = terminal
            .clone()
            .advance(ImportState::Received)
            .expect_err("a terminal run never advances");
        assert!(
            matches!(error, TransitionError::AlreadyTerminal { .. }),
            "from {terminal:?}"
        );
        // Terminality holds even toward sibling terminals such as failed.
        let error = terminal
            .clone()
            .advance(ImportState::Failed)
            .expect_err("a terminal run cannot be re-classified");
        assert!(matches!(error, TransitionError::AlreadyTerminal { .. }));
    }
}

#[test]
fn duplicate_is_entered_only_from_hashed() {
    let duplicated = ImportState::Hashed
        .advance(ImportState::Duplicate)
        .expect("the duplicate check fires exactly at hashed");
    assert_eq!(duplicated, ImportState::Duplicate);

    let error = ImportState::Stored
        .advance(ImportState::Duplicate)
        .expect_err("after storage the content is no longer a duplicate question");
    assert!(matches!(error, TransitionError::IllegalJump { .. }));
}

#[test]
fn states_spell_exactly_the_declared_set() {
    let declared = [
        ("received", ImportState::Received),
        ("hashed", ImportState::Hashed),
        ("stored", ImportState::Stored),
        ("inspected", ImportState::Inspected),
        ("parsed", ImportState::Parsed),
        ("reconciled", ImportState::Reconciled),
        ("completed", ImportState::Completed),
        ("partial", ImportState::Partial),
        ("failed", ImportState::Failed),
        ("duplicate", ImportState::Duplicate),
        ("quarantined", ImportState::Quarantined),
    ];
    for (spelling, state) in declared {
        assert_eq!(state.as_str(), spelling);
        assert_eq!(ImportState::parse(spelling), Some(state));
    }
    assert_eq!(ImportState::parse("unknown_stage"), None);
    assert_eq!(
        ImportState::parse("RECEIVED"),
        None,
        "spelling is lowercase"
    );
}
