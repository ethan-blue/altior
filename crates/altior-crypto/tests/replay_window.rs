//! Replay-window edge tests (ADR 0011): the boundaries the envelope
//! path cannot produce on its own (counter 0, full-window jumps).

use altior_crypto::ReplayWindow;

#[test]
fn zero_is_never_accepted() {
    let mut window = ReplayWindow::default();
    assert!(!window.accept(0), "counter 0 is reserved and invalid");
    window.accept(1);
    assert!(
        !window.accept(0),
        "counter 0 stays invalid even after real traffic"
    );
}

#[test]
fn duplicates_and_reordering_follow_the_window_rules() {
    let mut window = ReplayWindow::default();
    assert!(window.accept(5), "fresh highest");
    assert!(window.accept(3), "reordered within 64");
    assert!(!window.accept(3), "duplicate");
    assert!(window.accept(4), "gap filled later");
    assert!(!window.accept(4), "and its duplicate too");
}

#[test]
fn a_jump_of_64_resets_the_window() {
    let mut window = ReplayWindow::default();
    assert!(window.accept(1));
    // A jump of exactly 64 clears every remembered bit.
    assert!(window.accept(65));
    // 65 - 1 = 64: outside the window, refused even though unseen.
    assert!(!window.accept(1));
    // 65 - 2 = 63: the oldest delivery still inside the window.
    assert!(window.accept(2));
}

#[test]
fn a_jump_beyond_64_forgets_everything_old() {
    let mut window = ReplayWindow::default();
    assert!(window.accept(10));
    assert!(window.accept(20));
    assert!(window.accept(200));
    assert!(!window.accept(10), "far too old");
    assert!(!window.accept(20), "far too old");
    assert!(window.accept(200 - 63), "63 behind is still fresh");
}
