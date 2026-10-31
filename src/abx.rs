//! The ABX listening test: ten blind trials between two tracks.
//!
//! Both tracks are known to the listener as A and B. Each trial picks one of
//! them as X without saying which, and the listener says which of the two X
//! is. The answer is scored, and a test runs ten trials.
//!
//! A listener who cannot tell the tracks apart still guesses right half the
//! time, so the count of right answers follows a binomial distribution: ten
//! trials, a chance of one half each. Nine right answers or more come up by
//! guessing 11 times in 1024, about 1.1%, which is under the 5% a test is
//! usually held to; eight (5.5%) is not, which is why nine is the count a
//! test has to meet.

use std::path::PathBuf;
use std::time::Duration;

use crate::library::Track;

/// Trials a test runs, every one of them scored.
pub const TRIALS: usize = 10;

/// Right answers a listener needs to show they can tell the tracks apart.
///
/// At ten trials this is the smallest count under the usual 5% level: nine
/// or more right answers are guessed 11 times in 1024 (1.1%), eight or more
/// 56 times in 1024 (5.5%).
pub const PASSING: usize = 9;

/// One of the two tracks under test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The track labelled A.
    A,
    /// The track labelled B.
    B,
}

impl Side {
    /// The letter the window shows for this side.
    pub fn label(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
        }
    }
}

/// One of the three things the player can be offering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// The track labelled A.
    A,
    /// The track labelled B.
    B,
    /// The hidden track under test.
    X,
}

impl Choice {
    /// The letter the window shows for this choice.
    pub fn label(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::X => "X",
        }
    }
}

/// A test in progress.
pub struct Abx {
    /// The track labelled A.
    a: Track,
    /// The track labelled B.
    b: Track,
    /// Which of the two X is on the trial under way.
    x: Side,
    /// Whether each finished trial was answered right.
    results: Vec<bool>,
    /// The answer given for the trial under way, once there is one.
    answered: Option<Side>,
    /// What the player was last sent to, when it is offering anything.
    playing: Option<Choice>,
    /// State of the generator that picks X, so a test is not repeatable.
    seed: u64,
}

impl Abx {
    /// Starts a test between `a` and `b`, with X drawn from `seed`.
    ///
    /// `seed` has to differ between runs for X to be unpredictable; the app
    /// draws it from its own generator.
    pub fn new(a: Track, b: Track, seed: u64) -> Self {
        let mut test = Self {
            a,
            b,
            // Replaced by the draw below, which needs the rest of the state.
            x: Side::A,
            results: Vec::new(),
            answered: None,
            playing: None,
            // A zero seed would leave the generator stuck, so a caller that
            // hands one over still gets a usable test.
            seed: seed.max(1),
        };
        test.x = test.draw();
        test
    }

    /// The track on `side`.
    pub fn track(&self, side: Side) -> &Track {
        match side {
            Side::A => &self.a,
            Side::B => &self.b,
        }
    }

    /// The track `choice` names, with X standing for the side drawn now.
    pub fn choice(&self, choice: Choice) -> &Track {
        match choice {
            Choice::A => &self.a,
            Choice::B => &self.b,
            Choice::X => self.track(self.x),
        }
    }

    /// The file of `choice`, which is what the player opens.
    pub fn path(&self, choice: Choice) -> PathBuf {
        self.choice(choice).path.clone()
    }

    /// The length `choice` claims in its tags, for before the decoder says.
    pub fn length(&self, choice: Choice) -> Duration {
        self.choice(choice).duration
    }

    /// Which side X is on, during a trial and after it is answered.
    pub fn x(&self) -> Side {
        self.x
    }

    /// The number of the trial under way, counting from one.
    ///
    /// Once the last trial is answered this stays at [`TRIALS`], since there
    /// is no next one to name.
    pub fn trial(&self) -> usize {
        let next = self.results.len() + usize::from(self.answered.is_none());
        next.clamp(1, TRIALS)
    }

    /// Trials finished so far.
    pub fn finished(&self) -> usize {
        self.results.len()
    }

    /// The answer to the trial under way, while it is being shown.
    pub fn answered(&self) -> Option<Side> {
        self.answered
    }

    /// Whether the trial under way has been answered right.
    ///
    /// `None` until the listener says which side X is.
    pub fn answered_right(&self) -> Option<bool> {
        self.answered.map(|guess| guess == self.x)
    }

    /// Scores the trial under way against `guess`.
    ///
    /// Answers whether the guess was right, or `None` when the trial is
    /// already answered or the test is over: an answer that is taken twice
    /// would count twice.
    pub fn answer(&mut self, guess: Side) -> Option<bool> {
        if self.done() || self.answered.is_some() {
            return None;
        }
        let right = guess == self.x;
        self.results.push(right);
        self.answered = Some(guess);
        Some(right)
    }

    /// Moves to the trial after the one answered, drawing a new X.
    pub fn next(&mut self) {
        if self.answered.is_none() || self.done() {
            return;
        }
        self.answered = None;
        self.x = self.draw();
    }

    /// Whether every trial has been answered.
    pub fn done(&self) -> bool {
        self.results.len() >= TRIALS
    }

    /// Trials answered right.
    pub fn correct(&self) -> usize {
        self.results.iter().filter(|&&right| right).count()
    }

    /// Trials answered wrong.
    pub fn wrong(&self) -> usize {
        self.results.len() - self.correct()
    }

    /// Whether the listener showed they can tell the tracks apart.
    pub fn passed(&self) -> bool {
        self.correct() >= PASSING
    }

    /// The chance that guessing would have scored this well or better.
    ///
    /// This is the upper tail of a binomial with [`TRIALS`] trials and a
    /// chance of one half, for the right answers so far: the p-value an ABX
    /// test reports.
    pub fn p_value(&self) -> f64 {
        let ways: u32 = (self.correct()..=TRIALS)
            .map(|k| combinations(TRIALS, k))
            .sum();
        f64::from(ways) / (1_u32 << TRIALS) as f64
    }

    /// Starts the test again with the same two tracks, from a new `seed`.
    pub fn restart(&mut self, seed: u64) {
        self.results.clear();
        self.answered = None;
        self.playing = None;
        self.seed = seed.max(1);
        self.x = self.draw();
    }

    /// What the player was last sent to, when it is offering anything.
    pub fn playing(&self) -> Option<Choice> {
        self.playing
    }

    /// Records that the player was sent to `playing`.
    pub fn set_playing(&mut self, playing: Option<Choice>) {
        self.playing = playing;
    }

    /// Draws the side X takes on the next trial.
    ///
    /// The generator is the plain xorshift the shuffle uses, so no
    /// dependency is needed for two outcomes.
    fn draw(&mut self) -> Side {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        if self.seed & 1 == 0 { Side::A } else { Side::B }
    }
}

impl From<Side> for Choice {
    /// The choice that plays the track on `side`.
    fn from(side: Side) -> Self {
        match side {
            Side::A => Self::A,
            Side::B => Self::B,
        }
    }
}

/// How many ways `k` of `n` can be chosen, which the binomial needs.
///
/// `n` is ten here, so the result fits a `u32` many times over.
fn combinations(n: usize, k: usize) -> u32 {
    if k > n {
        return 0;
    }
    let k = k.min(n - k);
    (1..=k).fold(1_u32, |ways, i| ways * (n - k + i) as u32 / i as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A track that only needs a name, since a test never opens one.
    fn track(title: &str) -> Track {
        Track {
            path: PathBuf::from(format!("/music/{title}.flac")),
            title: title.to_string(),
            artist: "Someone".to_string(),
            album: "An Album".to_string(),
            album_artist: "Someone".to_string(),
            year: Some(2007),
            disc: None,
            number: Some(1),
            duration: Duration::from_secs(200),
            cover: None,
            directory: PathBuf::from("/music"),
            tagged_album: true,
        }
    }

    fn test(seed: u64) -> Abx {
        Abx::new(track("a"), track("b"), seed)
    }

    /// The side `side` is not on.
    fn other(side: Side) -> Side {
        match side {
            Side::A => Side::B,
            Side::B => Side::A,
        }
    }

    /// Answers `trials` trials, right until `right` are used up.
    fn score(test: &mut Abx, right: usize) {
        for index in 0..TRIALS {
            let x = test.x();
            let guess = if index < right { x } else { other(x) };
            test.answer(guess).expect("the trial was unanswered");
            test.next();
        }
    }

    #[test]
    fn a_test_runs_ten_trials_and_ends() {
        let mut test = test(1);
        assert_eq!(test.trial(), 1);
        for trial in 1..=TRIALS {
            assert_eq!(test.trial(), trial);
            test.answer(Side::A);
            assert!(!test.done() || trial == TRIALS);
            test.next();
        }
        assert!(test.done());
        assert_eq!(test.finished(), TRIALS);
        assert_eq!(test.correct() + test.wrong(), TRIALS);
    }

    #[test]
    fn nine_of_ten_answers_passes_and_eight_does_not() {
        let mut passing = test(1);
        score(&mut passing, 9);
        assert!(passing.passed());

        let mut failing = test(1);
        score(&mut failing, 8);
        assert!(!failing.passed());
    }

    #[test]
    fn a_score_is_judged_by_how_likely_guessing_would_reach_it() {
        // Nine or ten right answers out of ten are guessed eleven times in
        // 1024; eight or more, 56 times in 1024. The 5% level sits between.
        let mut test = test(1);
        score(&mut test, 9);
        assert_eq!(test.p_value(), 11.0 / 1024.0);
        assert!(test.p_value() < 0.05);

        let mut test = self::test(2);
        score(&mut test, 8);
        assert_eq!(test.p_value(), 56.0 / 1024.0);
        assert!(test.p_value() > 0.05);
    }

    #[test]
    fn a_trial_is_scored_once_and_only_while_it_is_open() {
        let mut test = test(3);
        assert!(test.answered_right().is_none());
        assert!(test.answer(test.x()).is_some());
        assert_eq!(test.answered_right(), Some(true));
        assert!(
            test.answer(test.x()).is_none(),
            "a second answer would count twice"
        );
        assert_eq!(test.finished(), 1);
    }

    #[test]
    fn the_answer_says_which_side_x_was() {
        let mut test = test(4);
        let x = test.x();
        assert_eq!(test.answered(), None);
        test.answer(other(x));
        assert_eq!(test.answered(), Some(other(x)));
        assert_eq!(test.answered_right(), Some(false));
        assert_eq!(test.x(), x, "the reveal does not move X");
    }

    #[test]
    fn a_new_trial_draws_x_again_and_forgets_the_answer() {
        let mut test = test(5);
        test.answer(Side::A);
        test.next();
        assert_eq!(test.answered(), None);
        assert_eq!(test.trial(), 2);
    }

    #[test]
    fn x_falls_on_both_sides_over_a_run_of_trials() {
        // Two outcomes drawn independently: over ten trials of a few seeds
        // both sides have to come up, or the draw is stuck on one.
        let mut sides = [false; 2];
        for seed in 1..=8 {
            let mut test = test(seed);
            for _ in 0..TRIALS {
                sides[usize::from(test.x() == Side::B)] = true;
                test.answer(test.x());
                test.next();
            }
        }
        assert_eq!(sides, [true, true]);
    }

    #[test]
    fn restarting_clears_the_score_and_asks_a_new_question() {
        let mut test = test(6);
        score(&mut test, 10);
        assert!(test.done());
        test.restart(7);
        assert!(!test.done());
        assert_eq!(test.correct(), 0);
        assert_eq!(test.finished(), 0);
        assert_eq!(test.trial(), 1);
        assert_eq!(test.answered(), None);
    }

    #[test]
    fn a_player_switched_to_x_plays_the_side_x_stands_for() {
        let mut test = test(8);
        assert_eq!(test.path(Choice::X), test.path(test.x().into()));
        assert_eq!(test.path(Choice::A), PathBuf::from("/music/a.flac"));
        assert_eq!(test.path(Choice::B), PathBuf::from("/music/b.flac"));
        test.set_playing(Some(Choice::X));
        assert_eq!(test.playing(), Some(Choice::X));
    }

    #[test]
    fn the_combinations_the_p_value_is_built_from_are_exact() {
        assert_eq!(combinations(10, 0), 1);
        assert_eq!(combinations(10, 5), 252);
        assert_eq!(combinations(10, 10), 1);
        assert_eq!(combinations(4, 5), 0);
    }
}
