//! The cabinet, drawn as cells: cards, paytable, wallet and the risk ladder.

use cread_core::Cell;
use cread_core::Screen;

use crate::cards::{Card, Hand};
use crate::{LADDER, Phase, Poker, money};

const CARD_W: u32 = 7;
const CARD_H: u32 = 5;
const CARD_GAP: u32 = 1;
const CARD_Y: u32 = 7;
const PAPER: u8 = 15;
const CARD_INK: u8 = 0;
const CARD_HELD: u8 = 7;
const CARD_RED: u8 = 12;

pub(super) fn draw(poker: &Poker, screen: &mut Screen) {
    screen.fill(Cell::blank(poker.ink, poker.paper));
    if poker.phase == Phase::Risk {
        draw_risk_ladder(poker, screen);
        return;
    }

    draw_frame(screen, poker, "AMERICAN POKER");
    draw_paytable(poker, screen);
    draw_cards(poker, screen);
    draw_money(poker, screen);
    draw_message(poker, screen);
    draw_controls(screen, poker);

    if poker.phase == Phase::GameOver {
        draw_game_over(poker, screen);
    }
}

fn draw_frame(screen: &mut Screen, poker: &Poker, title: &str) {
    let cols = screen.cols();
    let rows = screen.rows();
    let bottom = rows.saturating_sub(2);
    for y in 1..bottom {
        screen.put(0, y, Cell::new('\u{2551}', poker.ink, poker.paper));
        screen.put(
            cols.saturating_sub(1),
            y,
            Cell::new('\u{2551}', poker.ink, poker.paper),
        );
    }
    screen.put(0, 0, Cell::new('\u{2554}', poker.ink, poker.paper));
    screen.put(
        cols.saturating_sub(1),
        0,
        Cell::new('\u{2557}', poker.ink, poker.paper),
    );
    screen.put(0, bottom, Cell::new('\u{255a}', poker.ink, poker.paper));
    screen.put(
        cols.saturating_sub(1),
        bottom,
        Cell::new('\u{255d}', poker.ink, poker.paper),
    );
    screen.hline(
        1,
        0,
        cols.saturating_sub(2),
        '\u{2550}',
        poker.ink,
        poker.paper,
    );
    screen.hline(
        1,
        bottom,
        cols.saturating_sub(2),
        '\u{2550}',
        poker.ink,
        poker.paper,
    );

    let title = format!(" {title} ");
    let x = cols.saturating_sub(title.chars().count() as u32) / 2;
    screen.put_str(x, 0, &title, poker.ink, poker.paper);
    screen.put_str(2, 0, "\u{263c}", poker.ink, poker.paper);
    screen.put_str(
        cols.saturating_sub(3),
        0,
        "\u{263c}",
        poker.ink,
        poker.paper,
    );
}

fn draw_paytable(poker: &Poker, screen: &mut Screen) {
    put_center(screen, 1, "PAYTABLE", poker.ink, poker.paper);
    let rows: [[(&str, u64); 3]; 3] = [
        [
            ("ROYAL FLUSH", 250),
            ("FULL HOUSE", 9),
            ("THREE OF A KIND", 3),
        ],
        [("STRAIGHT FLUSH", 50), ("FLUSH", 6), ("TWO PAIR", 2)],
        [
            ("FOUR OF A KIND", 25),
            ("STRAIGHT", 4),
            ("JACKS OR BETTER", 1),
        ],
    ];
    let highlight = poker.result.and_then(Hand::paytable_cell);
    for (row, entries) in rows.iter().enumerate() {
        for (col, &(name, value)) in entries.iter().enumerate() {
            let x = 3 + col as u32 * 25;
            let (fg, bg) = if highlight == Some((col, row)) {
                (poker.paper, poker.ink)
            } else {
                (poker.ink, poker.paper)
            };
            screen.put_str(x, 2 + row as u32, name, fg, bg);
            let value = value.to_string();
            screen.put_str(x + 20, 2 + row as u32, &value, fg, bg);
        }
    }
}

fn draw_cards(poker: &Poker, screen: &mut Screen) {
    let total = 5 * CARD_W + 4 * CARD_GAP;
    let x0 = screen.cols().saturating_sub(total) / 2;
    for index in 0..5usize {
        let x = x0 + index as u32 * (CARD_W + CARD_GAP);
        match poker.hand.as_ref() {
            Some(hand) => {
                draw_card(
                    screen,
                    x,
                    CARD_Y,
                    hand[index],
                    poker.held[index],
                    index,
                    poker.ink,
                    poker.paper,
                );
            }
            None => draw_card_back(screen, x, CARD_Y, poker.ink),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_card(
    screen: &mut Screen,
    x: u32,
    y: u32,
    card: Card,
    held: bool,
    index: usize,
    ink: u8,
    paper: u8,
) {
    let bg = if held { CARD_HELD } else { PAPER };
    for py in 0..CARD_H {
        for px in 0..CARD_W {
            screen.put(x + px, y + py, Cell::blank(CARD_INK, bg));
        }
    }
    screen.put_str(
        x,
        y,
        "\u{250c}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2510}",
        CARD_INK,
        bg,
    );
    screen.put_str(
        x,
        y + 4,
        "\u{2514}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2518}",
        CARD_INK,
        bg,
    );
    for py in 1..4 {
        screen.put(x, y + py, Cell::new('\u{2502}', CARD_INK, bg));
        screen.put(x + CARD_W - 1, y + py, Cell::new('\u{2502}', CARD_INK, bg));
    }

    let red = if card.suit.red() { CARD_RED } else { CARD_INK };
    screen.put_str(x + 1, y + 1, card.rank.label(), red, bg);
    screen.put_str(x + 3, y + 2, &card.suit.glyph().to_string(), red, bg);
    let rank = card.rank.label();
    let rank_x = x + CARD_W - 1 - rank.chars().count() as u32;
    screen.put_str(rank_x, y + 3, rank, red, bg);

    if held {
        screen.put_str(x + 1, y.saturating_sub(1), "HELD", CARD_RED, paper);
    }
    screen.put_str(x + 3, y + CARD_H, &(index + 1).to_string(), ink, paper);
}

fn draw_card_back(screen: &mut Screen, x: u32, y: u32, ink: u8) {
    for py in 0..CARD_H {
        for px in 0..CARD_W {
            let edge = py == 0 || py == CARD_H - 1 || px == 0 || px == CARD_W - 1;
            let cell = if edge {
                Cell::blank(ink, CARD_HELD)
            } else {
                Cell::blank(8, CARD_HELD)
            };
            screen.put(x + px, y + py, cell);
        }
    }
    screen.put_str(
        x,
        y,
        "\u{250c}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2510}",
        ink,
        CARD_HELD,
    );
    screen.put_str(
        x,
        y + 4,
        "\u{2514}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2518}",
        ink,
        CARD_HELD,
    );
    for py in 1..4 {
        screen.put(x, y + py, Cell::new('\u{2502}', ink, CARD_HELD));
        screen.put(
            x + CARD_W - 1,
            y + py,
            Cell::new('\u{2502}', ink, CARD_HELD),
        );
    }
    for py in 1..4 {
        for px in 1..CARD_W - 1 {
            screen.put(x + px, y + py, Cell::new('\u{2592}', 8, CARD_HELD));
        }
    }
}

fn draw_money(poker: &Poker, screen: &mut Screen) {
    let line = format!(
        "BALANCE {}   BET {}   WIN {}   BEST {}",
        money(poker.balance),
        money(poker.bet),
        money(poker.win),
        money(poker.best)
    );
    put_center(screen, 14, &line, poker.ink, poker.paper);
}

fn draw_message(poker: &Poker, screen: &mut Screen) {
    let (line, fg) = match poker.phase {
        Phase::Ready => ("PRESS ENTER TO DEAL".to_string(), poker.ink),
        Phase::Dealt => ("HOLD 1-5   THEN ENTER TO DRAW".to_string(), poker.ink),
        Phase::Result => match poker.result {
            Some(hand) if poker.win > 0 => {
                let advice = if poker.risk_up().is_some() {
                    "T TAKE   R RISK"
                } else {
                    "T TAKE   TOP OF THE LADDER"
                };
                (
                    format!("{}   WIN {}   {}", hand.name(), money(poker.win), advice),
                    poker.ink,
                )
            }
            _ => ("NO WIN   ENTER FOR THE NEXT HAND".to_string(), poker.ink),
        },
        Phase::GameOver => (String::new(), poker.ink),
        Phase::Risk => (String::new(), poker.ink),
    };
    if !line.is_empty() {
        put_center(screen, 16, &line, fg, poker.paper);
    }
}

fn draw_controls(screen: &mut Screen, poker: &Poker) {
    put_center(
        screen,
        20,
        "1-5 HOLD   ENTER DEAL/DRAW   B BET   R RISK   T TAKE",
        poker.ink,
        poker.paper,
    );
    put_center(
        screen,
        21,
        "C CASH OUT   M MUTE   ESC COMMANDS",
        poker.ink,
        poker.paper,
    );
}

fn draw_game_over(poker: &Poker, screen: &mut Screen) {
    let left = 24u32;
    let right = 56u32;
    let top = 8u32;
    let bottom = 16u32;
    for y in top..=bottom {
        for x in left..=right {
            screen.put(x, y, Cell::blank(poker.ink, poker.paper));
        }
    }
    screen.put_str(left, top, "\u{2554}", poker.ink, poker.paper);
    screen.put_str(right, top, "\u{2557}", poker.ink, poker.paper);
    screen.put_str(left, bottom, "\u{255a}", poker.ink, poker.paper);
    screen.put_str(right, bottom, "\u{255d}", poker.ink, poker.paper);
    screen.hline(
        left + 1,
        top,
        right - left - 1,
        '\u{2550}',
        poker.ink,
        poker.paper,
    );
    screen.hline(
        left + 1,
        bottom,
        right - left - 1,
        '\u{2550}',
        poker.ink,
        poker.paper,
    );
    for y in top + 1..bottom {
        screen.put(left, y, Cell::new('\u{2551}', poker.ink, poker.paper));
        screen.put(right, y, Cell::new('\u{2551}', poker.ink, poker.paper));
    }
    put_center(screen, top + 2, "GAME OVER", poker.ink, poker.paper);
    put_center(
        screen,
        top + 4,
        &format!("BALANCE {}", money(poker.balance)),
        poker.ink,
        poker.paper,
    );
    put_center(
        screen,
        top + 5,
        &format!("HIGHSCORE {}", money(poker.best)),
        poker.ink,
        poker.paper,
    );
    put_center(
        screen,
        top + 7,
        "N NEW GAME   C CASH OUT",
        poker.ink,
        poker.paper,
    );
}

fn draw_risk_ladder(poker: &Poker, screen: &mut Screen) {
    draw_frame(screen, poker, "RISK LADDER");
    let up = poker.risk_up();
    let down = poker.risk_down();
    let high = poker.risk_blink();
    put_center(
        screen,
        1,
        &format!(
            "WIN {}   RISK {} / {}",
            money(poker.win),
            up.map(money).unwrap_or_else(|| "TOP".into()),
            down.map(money).unwrap_or_else(|| "0.00".into())
        ),
        poker.ink,
        poker.paper,
    );

    let current = poker.win;
    for (index, &step) in LADDER.iter().enumerate().rev() {
        let y = 2 + (LADDER.len() - 1 - index) as u32;
        let is_up = Some(step) == up;
        let is_down = Some(step) == down;
        let lit = (is_up && high) || (is_down && !high);
        let (fg, bg) = if lit {
            (poker.paper, poker.ink)
        } else if step == 0 {
            (CARD_RED, poker.paper)
        } else {
            (poker.ink, poker.paper)
        };
        let marker = if step == current { ">" } else { " " };
        screen.put_str(28, y, marker, fg, bg);
        let amount = money(step);
        screen.put_str(30, y, &amount, fg, bg);
        if is_up {
            screen.put_str(48, y, "UP", fg, bg);
        } else if is_down {
            screen.put_str(48, y, "DOWN", fg, bg);
        }
    }

    let hint = if poker.win == 0 {
        "THE WIN IS GONE   ENTER TO CONTINUE"
    } else if up.is_none() {
        "TOP OF THE LADDER   T TAKE YOUR WIN"
    } else {
        "ENTER RISK   T TAKE   C CASH OUT   M MUTE   ESC COMMANDS"
    };
    put_center(screen, 22, hint, poker.ink, poker.paper);
}

fn put_center(screen: &mut Screen, y: u32, text: &str, fg: u8, bg: u8) {
    let width = text.chars().count() as u32;
    let x = screen.cols().saturating_sub(width) / 2;
    screen.put_str(x, y, text, fg, bg);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::{Card, Rank, Suit};
    use crate::save::Wallet;
    use std::path::PathBuf;

    fn game(tag: &str) -> Poker {
        let dir = std::env::temp_dir().join(format!("cread-poker-ui-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Poker::for_test(
            Wallet {
                balance: 1_000,
                best: 2_000,
            },
            dir.join("poker.save"),
            7,
        )
    }

    fn rows(screen: &Screen) -> String {
        screen
            .cells()
            .iter()
            .map(|cell| cell.ch)
            .collect::<String>()
    }

    #[test]
    fn the_ready_screen_shows_the_cabinet_and_the_wallet() {
        let poker = game("ready");
        let mut screen = Screen::new(80, 25);
        draw(&poker, &mut screen);
        let text = rows(&screen);
        assert!(text.contains("AMERICAN POKER"));
        assert!(text.contains("BALANCE $10.00"));
        assert!(text.contains("PRESS ENTER TO DEAL"));
        assert!(text.contains("PAYTABLE"));
        let _ = std::fs::remove_dir_all(PathBuf::from(poker.save_path.parent().unwrap()));
    }

    #[test]
    fn the_cards_show_cp437_suits_and_held_labels() {
        let mut poker = game("cards");
        poker.hand = Some([
            Card::new(Rank::Ace, Suit::Spades),
            Card::new(Rank::Ten, Suit::Hearts),
            Card::new(Rank::King, Suit::Diamonds),
            Card::new(Rank::Three, Suit::Clubs),
            Card::new(Rank::Seven, Suit::Spades),
        ]);
        poker.held[0] = true;
        poker.phase = Phase::Dealt;
        let mut screen = Screen::new(80, 25);
        draw(&poker, &mut screen);
        let text = rows(&screen);
        assert!(text.contains('\u{2660}'));
        assert!(text.contains('\u{2665}'));
        assert!(text.contains('\u{2666}'));
        assert!(text.contains('\u{2663}'));
        assert!(text.contains("HELD"));
        let _ = std::fs::remove_dir_all(PathBuf::from(poker.save_path.parent().unwrap()));
    }

    #[test]
    fn the_result_highlights_the_hand_and_the_take_options() {
        let mut poker = game("result");
        poker.phase = Phase::Result;
        poker.result = Some(Hand::Flush);
        poker.win = 60;
        let mut screen = Screen::new(80, 25);
        draw(&poker, &mut screen);
        let text = rows(&screen);
        assert!(text.contains("FLUSH"));
        assert!(text.contains("WIN $0.60"));
        assert!(text.contains("T TAKE"));
        assert!(text.contains("R RISK"));
        let _ = std::fs::remove_dir_all(PathBuf::from(poker.save_path.parent().unwrap()));
    }

    #[test]
    fn game_over_offers_a_new_game_and_a_cash_out() {
        let mut poker = game("over");
        poker.balance = 0;
        poker.phase = Phase::GameOver;
        let mut screen = Screen::new(80, 25);
        draw(&poker, &mut screen);
        let text = rows(&screen);
        assert!(text.contains("GAME OVER"));
        assert!(text.contains("N NEW GAME"));
        assert!(text.contains("C CASH OUT"));
        let _ = std::fs::remove_dir_all(PathBuf::from(poker.save_path.parent().unwrap()));
    }

    #[test]
    fn the_risk_ladder_shows_the_exact_steps() {
        let mut poker = game("risk");
        poker.phase = Phase::Risk;
        poker.win = 90;
        let mut screen = Screen::new(80, 25);
        draw(&poker, &mut screen);
        let text = rows(&screen);
        assert!(text.contains("RISK LADDER"));
        assert!(text.contains("$140.00"));
        assert!(text.contains("$84.00"));
        assert!(text.contains("$0.00"));
        assert!(text.contains("$1.20"));
        assert!(text.contains("$0.80"));
        assert!(text.contains("$0.15"));
        let _ = std::fs::remove_dir_all(PathBuf::from(poker.save_path.parent().unwrap()));
    }
}
