//! US Poker: a five-card-draw machine with a bankroll and a risk ladder.
//!
//! The shell owns the screen and the prompt; this crate owns the cabinet, the
//! cards, the money and the little speaker. The bankroll is machine state, not
//! a document, so it lives in `~/.config/cread/poker.save` and never enters the
//! vault or WebDAV.

mod cards;
mod save;
mod sfx;
mod ui;

use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use cread_core::command::Command;
use cread_core::program::{KeyPress, Outcome, PointerEvent, Program};
use cread_core::{Raster, Screen};

use cards::{Card, Deck, Hand, Rng, evaluate};
use sfx::{Effect, Sfx};

pub use save::{MIN_BET, START_BALANCE, Wallet, money};

/// The bankroll as the front door sees it.
pub fn wallet() -> Wallet {
    save::load()
}

/// The ladder, in cents, low to high. Exactly the steps the machine shows.
pub(crate) const LADDER: [u64; 20] = [
    0, 15, 30, 40, 60, 80, 120, 160, 240, 300, 400, 600, 800, 1_200, 1_600, 2_400, 3_200, 4_800,
    8_400, 14_000,
];

/// The bets the button steps through, in cents.
const BETS: [u64; 8] = [10, 20, 50, 100, 200, 500, 1_000, 2_000];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    /// No hand in front of the player yet.
    Ready,
    /// Five cards up, waiting for holds or a draw.
    Dealt,
    /// The hand has been drawn and paid.
    Result,
    /// The risk ladder is in front.
    Risk,
    /// The wallet cannot afford the minimum bet.
    GameOver,
}

pub struct Poker {
    pub(crate) balance: u64,
    pub(crate) best: u64,
    pub(crate) bankrupts: u64,
    /// The current bust has already been counted. Keeps `exit` + drop from
    /// counting the same bankruptcy twice.
    bust_counted: bool,
    pub(crate) bet: u64,
    pub(crate) hand: Option<[Card; 5]>,
    pub(crate) held: [bool; 5],
    deck: Option<Deck>,
    pub(crate) phase: Phase,
    pub(crate) result: Option<Hand>,
    pub(crate) win: u64,
    /// When the current risk round started; the two fields blink from here.
    pub(crate) risk_started: Instant,
    rng: Rng,
    sound: Option<Sfx>,
    pub(crate) muted: bool,
    pub(crate) paper: u8,
    pub(crate) ink: u8,
    save_path: PathBuf,
    /// Real games cash out in `Drop`; unit tests and demo tables opt out.
    persist_on_drop: bool,
}

impl Default for Poker {
    fn default() -> Self {
        Self::new()
    }
}

impl Poker {
    pub fn new() -> Self {
        Self::new_at(save::path())
    }

    /// Open a machine with a specific save file. The app uses this in tests so
    /// they never touch the real wallet.
    pub fn new_at(save_path: PathBuf) -> Self {
        let wallet = save::load_from(&save_path);
        Self::build(wallet, save_path, seed(), true, true)
    }

    fn build(
        wallet: Wallet,
        save_path: PathBuf,
        seed: u64,
        sound: bool,
        persist_on_drop: bool,
    ) -> Self {
        let balance = wallet.balance;
        let best = wallet.best.max(balance);
        let busted = balance < MIN_BET;
        Self {
            balance,
            best,
            bankrupts: wallet.bankrupts,
            bust_counted: busted,
            bet: MIN_BET,
            hand: None,
            held: [false; 5],
            deck: None,
            phase: if balance < MIN_BET {
                Phase::GameOver
            } else {
                Phase::Ready
            },
            result: None,
            win: 0,
            risk_started: Instant::now(),
            rng: Rng::new(seed),
            sound: sound.then(Sfx::new),
            muted: false,
            paper: 15,
            ink: 0,
            save_path,
            persist_on_drop,
        }
    }

    #[cfg(test)]
    fn for_test(wallet: Wallet, save_path: PathBuf, seed: u64) -> Self {
        Self::build(wallet, save_path, seed, false, false)
    }

    /// A deterministic table for `--show pokerhand`; it never touches the real
    /// wallet.
    pub fn demo_hand() -> Self {
        let path = std::env::temp_dir().join("cread-poker-demo.save");
        let mut poker = Self::build(Wallet::default(), path, 7, false, false);
        poker.start_hand();
        // A fixed, readable hand for screenshots; the real machine shuffles.
        poker.hand = Some([
            Card::new(cards::Rank::Ace, cards::Suit::Spades),
            Card::new(cards::Rank::King, cards::Suit::Hearts),
            Card::new(cards::Rank::Queen, cards::Suit::Diamonds),
            Card::new(cards::Rank::Jack, cards::Suit::Clubs),
            Card::new(cards::Rank::Ten, cards::Suit::Spades),
        ]);
        poker
    }

    /// The same, with a win on the ladder, for `--show pokerrisk`.
    pub fn demo_risk() -> Self {
        let mut poker = Self::demo_hand();
        poker.phase = Phase::Result;
        poker.win = 90;
        poker.start_risk();
        poker
    }

    /// A bankrupt table for `--show pokerover`; the real wallet is untouched.
    pub fn demo_bust() -> Self {
        let mut poker = Self::demo_hand();
        poker.balance = 0;
        poker.win = 0;
        poker.phase = Phase::GameOver;
        poker.bankrupts = 3;
        poker
    }

    /// Raise the bet to the next affordable step, wrapping back to the minimum.
    pub(crate) fn cycle_bet(&mut self) {
        if self.phase != Phase::Ready {
            return;
        }
        let next = BETS
            .iter()
            .copied()
            .find(|&bet| bet > self.bet && bet <= self.balance)
            .or_else(|| BETS.iter().copied().find(|&bet| bet <= self.balance))
            .unwrap_or(MIN_BET);
        self.bet = next;
    }

    /// Keep the bet inside what is left after a hand.
    fn clamp_bet(&mut self) {
        if self.bet <= self.balance {
            return;
        }
        self.bet = BETS
            .iter()
            .rev()
            .copied()
            .find(|&bet| bet <= self.balance)
            .unwrap_or(MIN_BET);
    }

    fn start_hand(&mut self) {
        if self.phase != Phase::Ready {
            return;
        }
        self.clamp_bet();
        if self.balance < self.bet {
            self.count_bust();
            self.phase = Phase::GameOver;
            self.save();
            return;
        }
        self.balance -= self.bet;
        let mut deck = Deck::shuffled(&mut self.rng);
        let mut hand = [Card::new(cards::Rank::Ace, cards::Suit::Spades); 5];
        for card in &mut hand {
            if let Some(next) = deck.draw() {
                *card = next;
            }
        }
        self.hand = Some(hand);
        self.deck = Some(deck);
        self.held = [false; 5];
        self.result = None;
        self.win = 0;
        self.phase = Phase::Dealt;
        self.save();
        self.play(Effect::Deal);
    }

    fn draw_cards(&mut self) {
        if self.phase != Phase::Dealt {
            return;
        }
        let Some(hand) = self.hand.as_mut() else {
            return;
        };
        let Some(deck) = self.deck.as_mut() else {
            return;
        };
        for (index, card) in hand.iter_mut().enumerate() {
            if !self.held[index]
                && let Some(next) = deck.draw()
            {
                *card = next;
            }
        }
        let result = evaluate(hand);
        self.result = Some(result);
        self.win = result.multiplier() * self.bet;
        self.phase = Phase::Result;
        self.play(if self.win > 0 {
            Effect::Win
        } else {
            Effect::Loss
        });
    }

    fn take(&mut self) {
        if !matches!(self.phase, Phase::Result | Phase::Risk) {
            return;
        }
        if self.win > 0 {
            self.balance += self.win;
            self.win = 0;
            self.play(Effect::Coin);
        }
        let busted = self.balance < MIN_BET;
        if busted {
            self.count_bust();
        }
        self.result = None;
        self.held = [false; 5];
        self.clamp_bet();
        self.phase = if busted {
            Phase::GameOver
        } else {
            Phase::Ready
        };
        self.update_best();
        self.save();
    }

    fn start_risk(&mut self) {
        if self.phase != Phase::Result || self.win == 0 || self.risk_up().is_none() {
            return;
        }
        self.phase = Phase::Risk;
        self.risk_started = Instant::now();
    }

    pub(crate) fn risk_up(&self) -> Option<u64> {
        next_up(self.win)
    }

    pub(crate) fn risk_down(&self) -> Option<u64> {
        next_down(self.win)
    }

    pub(crate) fn risk_blink(&self) -> bool {
        (self.risk_started.elapsed().as_millis() / 140).is_multiple_of(2)
    }

    fn press_risk(&mut self) {
        if self.phase != Phase::Risk {
            return;
        }
        if self.win == 0 {
            self.take();
            return;
        }
        let Some(up) = self.risk_up() else {
            self.take();
            return;
        };
        if self.rng.chance(50) {
            self.win = up;
            self.play(Effect::RiskWin);
        } else {
            self.win = self.risk_down().unwrap_or(0);
            self.play(if self.win == 0 {
                Effect::Loss
            } else {
                Effect::RiskLoss
            });
        }
        self.risk_started = Instant::now();
    }

    fn toggle_hold(&mut self, index: usize) {
        if self.phase != Phase::Dealt || index >= self.held.len() {
            return;
        }
        self.held[index] = !self.held[index];
        self.play(Effect::Hold);
    }

    fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        if let Some(sound) = self.sound.as_mut() {
            sound.set_muted(self.muted);
        }
    }

    fn advance(&mut self) {
        match self.phase {
            Phase::Ready => self.start_hand(),
            Phase::Dealt => self.draw_cards(),
            Phase::Result => self.take(),
            Phase::Risk => self.press_risk(),
            Phase::GameOver => {}
        }
    }

    fn new_game(&mut self) {
        self.balance = START_BALANCE;
        self.bet = MIN_BET;
        self.hand = None;
        self.held = [false; 5];
        self.deck = None;
        self.result = None;
        self.win = 0;
        self.bust_counted = false;
        self.phase = Phase::Ready;
        self.update_best();
        self.save();
        self.play(Effect::Coin);
    }

    /// Add a held win to the wallet, persist, and leave the table.
    pub fn cash_out(&mut self) {
        if matches!(self.phase, Phase::Result | Phase::Risk) && self.win > 0 {
            self.balance += self.win;
            self.win = 0;
        }
        if self.balance < MIN_BET {
            self.count_bust();
        }
        self.update_best();
        self.save();
    }

    fn count_bust(&mut self) {
        if self.balance < MIN_BET && !self.bust_counted {
            self.bankrupts += 1;
            self.bust_counted = true;
        }
    }

    fn update_best(&mut self) {
        self.best = self.best.max(self.balance);
    }

    fn save(&self) {
        let wallet = Wallet {
            balance: self.balance,
            best: self.best,
            bankrupts: self.bankrupts,
        };
        let _ = save::save_to(&self.save_path, &wallet);
    }

    fn play(&mut self, effect: Effect) {
        if let Some(sound) = self.sound.as_mut() {
            sound.play(effect);
        }
    }
}

impl Drop for Poker {
    fn drop(&mut self) {
        if self.persist_on_drop {
            self.cash_out();
        }
    }
}

impl Program for Poker {
    fn title(&self) -> &str {
        "US POKER"
    }

    fn key(&mut self, key: KeyPress) -> Outcome {
        match key {
            KeyPress::Char('c') | KeyPress::Char('C') => {
                self.cash_out();
                if self.balance < MIN_BET && self.phase != Phase::GameOver {
                    self.phase = Phase::GameOver;
                    return Outcome::Continue;
                }
                return Outcome::Exit;
            }
            KeyPress::Char('m') | KeyPress::Char('M') => self.toggle_mute(),
            KeyPress::Char('b') | KeyPress::Char('B') => self.cycle_bet(),
            KeyPress::Char('r') | KeyPress::Char('R') if self.phase == Phase::Result => {
                self.start_risk()
            }
            KeyPress::Char('r') | KeyPress::Char('R') if self.phase == Phase::Risk => {
                self.press_risk()
            }
            KeyPress::Char('t') | KeyPress::Char('T')
                if matches!(self.phase, Phase::Result | Phase::Risk) =>
            {
                self.take()
            }
            KeyPress::Char('y') | KeyPress::Char('Y') if self.phase == Phase::GameOver => {
                self.new_game()
            }
            KeyPress::Char('n') | KeyPress::Char('N') if self.phase == Phase::GameOver => {
                self.cash_out();
                return Outcome::Exit;
            }
            KeyPress::Char(ch @ '1'..='5') => self.toggle_hold(ch as usize - '1' as usize),
            KeyPress::Enter | KeyPress::Char(' ') => self.advance(),
            _ => {}
        }
        Outcome::Continue
    }

    fn pointer(&mut self, _event: PointerEvent) -> Outcome {
        Outcome::Continue
    }

    fn draw(&mut self, screen: &mut Screen, _raster: &mut Raster) {
        ui::draw(self, screen);
    }

    fn command(&mut self, command: &Command) -> Result<String, String> {
        match command {
            Command::Mute => {
                self.toggle_mute();
                Ok(if self.muted { "muted" } else { "sound on" }.into())
            }
            _ => Err("US POKER does not know that command".into()),
        }
    }

    fn status(&self) -> String {
        match self.phase {
            Phase::Ready => format!("balance {} · bet {}", money(self.balance), money(self.bet)),
            Phase::Dealt => format!("balance {} · hold 1-5", money(self.balance)),
            Phase::Result => {
                if self.win > 0 {
                    format!("win {} · T take or R risk", money(self.win))
                } else {
                    "no win · enter next".into()
                }
            }
            Phase::Risk => format!("risk · win {}", money(self.win)),
            Phase::GameOver => "bankrupt · Y retry · N cash out".into(),
        }
    }

    fn palette(&mut self, background: u8, ink: u8) {
        self.paper = background;
        self.ink = ink;
    }

    fn wants_frames(&self) -> bool {
        self.phase == Phase::Risk && self.win > 0 && self.risk_up().is_some()
    }

    fn exit(&mut self) -> bool {
        self.cash_out();
        false
    }
}

/// The next ladder step above `win`, or `None` at the top.
pub(crate) fn next_up(win: u64) -> Option<u64> {
    LADDER.iter().copied().find(|&step| step > win)
}

/// The next ladder step below `win`, or `None` when there is nothing below.
pub(crate) fn next_down(win: u64) -> Option<u64> {
    LADDER.iter().rev().copied().find(|&step| step < win)
}

fn seed() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0);
    nanos ^ (u64::from(std::process::id()) << 32) ^ 0x9e37_79b9_7f4a_7c15
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("cread-poker-state-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("poker.save")
    }

    fn game(tag: &str, balance: u64) -> Poker {
        Poker::for_test(
            Wallet {
                balance,
                best: balance,
                bankrupts: 0,
            },
            temp_path(tag),
            1234,
        )
    }

    #[test]
    fn the_ladder_is_exactly_the_machine_steps() {
        assert_eq!(LADDER.len(), 20);
        assert_eq!(LADDER[0], 0);
        assert_eq!(LADDER[1], 15);
        assert_eq!(LADDER[18], 8_400);
        assert_eq!(LADDER[19], 14_000);
        assert_eq!(next_up(10), Some(15));
        assert_eq!(next_down(10), Some(0));
        assert_eq!(next_up(90), Some(120));
        assert_eq!(next_down(90), Some(80));
        assert_eq!(next_up(14_000), None);
        assert_eq!(next_down(15), Some(0));
    }

    #[test]
    fn a_bet_only_moves_to_affordable_steps() {
        let mut poker = game("bet", 1_500);
        assert_eq!(poker.bet, 10);
        poker.cycle_bet();
        assert_eq!(poker.bet, 20);
        for _ in 0..10 {
            poker.cycle_bet();
            assert!(poker.bet <= poker.balance);
        }
        assert!(BETS.contains(&poker.bet));
    }

    #[test]
    fn a_win_is_held_until_it_is_taken() {
        let mut poker = game("take", 1_000);
        poker.phase = Phase::Result;
        poker.win = 120;
        poker.take();
        assert_eq!(poker.balance, 1_120);
        assert_eq!(poker.best, 1_120);
        assert_eq!(poker.phase, Phase::Ready);
        assert_eq!(poker.win, 0);

        let wallet = save::load_from(&poker.save_path);
        assert_eq!(wallet.balance, 1_120);
        assert_eq!(wallet.best, 1_120);
        let _ = std::fs::remove_dir_all(poker.save_path.parent().unwrap());
    }

    #[test]
    fn cashing_out_banks_a_held_win() {
        let mut poker = game("cash", 500);
        poker.phase = Phase::Result;
        poker.win = 240;
        poker.cash_out();
        assert_eq!(poker.balance, 740);
        let wallet = save::load_from(&poker.save_path);
        assert_eq!(wallet.balance, 740);
        assert_eq!(wallet.best, 740);
        let _ = std::fs::remove_dir_all(poker.save_path.parent().unwrap());
    }

    #[test]
    fn a_new_game_restores_ten_dollars_but_keeps_the_best() {
        let mut poker = game("new", 0);
        poker.best = 9_000;
        poker.bankrupts = 4;
        poker.phase = Phase::GameOver;
        poker.new_game();
        assert_eq!(poker.balance, START_BALANCE);
        assert_eq!(poker.best, 9_000);
        assert_eq!(poker.bankrupts, 4, "a retry never erases the count");
        assert_eq!(poker.phase, Phase::Ready);
        let _ = std::fs::remove_dir_all(poker.save_path.parent().unwrap());
    }

    #[test]
    fn a_hand_costs_the_bet_and_reaches_the_result() {
        let mut poker = game("hand", 1_000);
        poker.start_hand();
        assert_eq!(poker.balance, 990);
        assert_eq!(poker.phase, Phase::Dealt);
        assert!(poker.hand.is_some());
        poker.draw_cards();
        assert_eq!(poker.phase, Phase::Result);
        assert!(poker.result.is_some());
        let _ = std::fs::remove_dir_all(poker.save_path.parent().unwrap());
    }

    #[test]
    fn a_bust_counts_once_and_retry_keeps_it() {
        let mut poker = game("bust", 100);
        poker.phase = Phase::Result;
        poker.win = 0;
        poker.balance = 0;
        poker.take();
        assert_eq!(poker.phase, Phase::GameOver);
        assert_eq!(poker.bankrupts, 1);
        assert_eq!(
            save::load_from(&poker.save_path).bankrupts,
            1,
            "written when it happens"
        );

        poker.key(KeyPress::Char('y'));
        assert_eq!(poker.phase, Phase::Ready);
        assert_eq!(poker.balance, START_BALANCE);
        assert_eq!(poker.bankrupts, 1);

        poker.phase = Phase::GameOver;
        assert_eq!(poker.key(KeyPress::Char('n')), Outcome::Exit);
        assert_eq!(
            poker.bankrupts, 1,
            "leaving the lose screen is not a new bust"
        );
        let _ = std::fs::remove_dir_all(poker.save_path.parent().unwrap());
    }

    #[test]
    fn cashing_out_broke_counts_a_bankruptcy() {
        let mut poker = game("bust-cash", 100);
        poker.phase = Phase::Dealt;
        poker.balance = 0;
        poker.cash_out();
        assert_eq!(poker.bankrupts, 1);
        assert_eq!(save::load_from(&poker.save_path).bankrupts, 1);
        let _ = std::fs::remove_dir_all(poker.save_path.parent().unwrap());
    }

    #[test]
    fn a_saved_bust_is_not_counted_again_on_launch() {
        let path = temp_path("saved-bust");
        save::save_to(
            &path,
            &Wallet {
                balance: 0,
                best: 1_000,
                bankrupts: 5,
            },
        )
        .unwrap();
        let poker = Poker::for_test(save::load_from(&path), path.clone(), 1);
        assert_eq!(poker.phase, Phase::GameOver);
        assert_eq!(poker.bankrupts, 5);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_bet_only_changes_before_a_deal() {
        let mut poker = game("bet-phase", 1_000);
        poker.cycle_bet();
        assert_eq!(poker.bet, 20);
        poker.phase = Phase::Result;
        let before = poker.bet;
        poker.cycle_bet();
        assert_eq!(poker.bet, before, "a result keeps its original bet");
        let _ = std::fs::remove_dir_all(poker.save_path.parent().unwrap());
    }

    #[test]
    fn taking_a_hand_clears_the_held_markers() {
        let mut poker = game("held-clear", 1_000);
        poker.phase = Phase::Result;
        poker.held = [true, false, true, false, false];
        poker.win = 0;
        poker.take();
        assert_eq!(poker.held, [false; 5]);
        let _ = std::fs::remove_dir_all(poker.save_path.parent().unwrap());
    }

    #[test]
    fn dropping_the_table_banks_a_held_win() {
        let path = temp_path("drop-win");
        save::save_to(
            &path,
            &Wallet {
                balance: 1_000,
                best: 1_000,
                bankrupts: 0,
            },
        )
        .unwrap();
        let mut poker = Poker::new_at(path.clone());
        poker.phase = Phase::Result;
        poker.win = 120;
        drop(poker);

        let wallet = save::load_from(&path);
        assert_eq!(
            wallet.balance, 1_120,
            "closing the window must cash the win"
        );
        assert_eq!(wallet.best, 1_120);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn dropping_a_broke_table_counts_only_once() {
        let path = temp_path("drop-bust");
        save::save_to(
            &path,
            &Wallet {
                balance: 1_000,
                best: 1_000,
                bankrupts: 0,
            },
        )
        .unwrap();
        let mut poker = Poker::new_at(path.clone());
        poker.phase = Phase::Dealt;
        poker.balance = 0;
        poker.cash_out();
        poker.exit();
        drop(poker);

        let wallet = save::load_from(&path);
        assert_eq!(wallet.bankrupts, 1, "exit + drop is one bankruptcy");
        assert_eq!(wallet.balance, 0);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_keys_deal_hold_draw_and_cash_out() {
        let mut poker = game("keys", 1_000);
        assert_eq!(poker.key(KeyPress::Enter), Outcome::Continue);
        assert_eq!(poker.phase, Phase::Dealt);
        assert!(poker.hand.is_some());

        poker.key(KeyPress::Char('1'));
        assert!(poker.held[0]);
        poker.key(KeyPress::Char('1'));
        assert!(!poker.held[0]);

        poker.key(KeyPress::Enter);
        assert_eq!(poker.phase, Phase::Result);
        assert!(poker.result.is_some());

        assert_eq!(poker.key(KeyPress::Char('c')), Outcome::Exit);
        let wallet = save::load_from(&poker.save_path);
        assert_eq!(wallet.balance, poker.balance);
        let _ = std::fs::remove_dir_all(poker.save_path.parent().unwrap());
    }

    #[test]
    fn risk_starts_from_the_result_key_and_takes_the_win() {
        let mut poker = game("risk-key", 1_000);
        poker.phase = Phase::Result;
        poker.win = 90;
        poker.key(KeyPress::Char('r'));
        assert_eq!(poker.phase, Phase::Risk);
        poker.key(KeyPress::Char('t'));
        assert_eq!(poker.phase, Phase::Ready);
        assert_eq!(poker.balance, 1_090);
        let _ = std::fs::remove_dir_all(poker.save_path.parent().unwrap());
    }

    #[test]
    fn the_risk_ladder_moves_one_field_per_press() {
        let mut poker = game("risk", 1_000);
        poker.phase = Phase::Result;
        poker.win = 90;
        poker.start_risk();
        assert_eq!(poker.phase, Phase::Risk);
        let before = poker.win;
        poker.press_risk();
        let after = poker.win;
        assert!(
            after == next_up(before).unwrap() || after == next_down(before).unwrap(),
            "{before} -> {after} is not a ladder step"
        );
        let _ = std::fs::remove_dir_all(poker.save_path.parent().unwrap());
    }

    #[test]
    fn risk_cannot_start_without_a_win_or_above_the_top() {
        let mut poker = game("risk-no", 1_000);
        poker.phase = Phase::Result;
        poker.win = 0;
        poker.start_risk();
        assert_eq!(poker.phase, Phase::Result);
        poker.win = 14_000;
        poker.start_risk();
        assert_eq!(poker.phase, Phase::Result);
        let _ = std::fs::remove_dir_all(poker.save_path.parent().unwrap());
    }
}
