//! The deck, the draw, and a plain 9/6 Jacks-or-Better evaluator.

/// The four suits, in the order a deck is built.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Suit {
    Spades,
    Hearts,
    Diamonds,
    Clubs,
}

impl Suit {
    pub const ALL: [Suit; 4] = [Suit::Spades, Suit::Hearts, Suit::Diamonds, Suit::Clubs];

    pub fn glyph(self) -> char {
        match self {
            Suit::Spades => '\u{2660}',
            Suit::Hearts => '\u{2665}',
            Suit::Diamonds => '\u{2666}',
            Suit::Clubs => '\u{2663}',
        }
    }

    pub fn red(self) -> bool {
        matches!(self, Suit::Hearts | Suit::Diamonds)
    }
}

/// Two through ace; the value is also the handy sort key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rank {
    Two = 2,
    Three = 3,
    Four = 4,
    Five = 5,
    Six = 6,
    Seven = 7,
    Eight = 8,
    Nine = 9,
    Ten = 10,
    Jack = 11,
    Queen = 12,
    King = 13,
    Ace = 14,
}

impl Rank {
    pub const ALL: [Rank; 13] = [
        Rank::Two,
        Rank::Three,
        Rank::Four,
        Rank::Five,
        Rank::Six,
        Rank::Seven,
        Rank::Eight,
        Rank::Nine,
        Rank::Ten,
        Rank::Jack,
        Rank::Queen,
        Rank::King,
        Rank::Ace,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Rank::Two => "2",
            Rank::Three => "3",
            Rank::Four => "4",
            Rank::Five => "5",
            Rank::Six => "6",
            Rank::Seven => "7",
            Rank::Eight => "8",
            Rank::Nine => "9",
            Rank::Ten => "10",
            Rank::Jack => "J",
            Rank::Queen => "Q",
            Rank::King => "K",
            Rank::Ace => "A",
        }
    }

    pub fn value(self) -> u8 {
        self as u8
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Card {
    pub rank: Rank,
    pub suit: Suit,
}

impl Card {
    pub fn new(rank: Rank, suit: Suit) -> Self {
        Self { rank, suit }
    }
}

/// A small xorshift generator: no dependency, no crypto, plenty for cards.
#[derive(Clone, Debug)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self { state: seed | 1 }
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(2_685_821_657_736_338_717)
    }

    pub fn below(&mut self, limit: u64) -> u64 {
        if limit == 0 {
            return 0;
        }
        self.next_u64() % limit
    }

    /// True with `percent` chances out of a hundred.
    pub fn chance(&mut self, percent: u32) -> bool {
        self.below(100) < u64::from(percent.min(100))
    }
}

/// A shuffled 52-card deck. Each hand gets a fresh one.
pub struct Deck {
    cards: [Card; 52],
    next: usize,
}

impl Deck {
    pub fn shuffled(rng: &mut Rng) -> Self {
        let mut cards = Vec::with_capacity(52);
        for suit in Suit::ALL {
            for rank in Rank::ALL {
                cards.push(Card::new(rank, suit));
            }
        }
        let cards: [Card; 52] = cards.try_into().expect("a deck is 52 cards");
        let mut deck = Self { cards, next: 0 };
        for i in (1..deck.cards.len()).rev() {
            let j = rng.below(i as u64 + 1) as usize;
            deck.cards.swap(i, j);
        }
        deck
    }

    pub fn draw(&mut self) -> Option<Card> {
        let card = self.cards.get(self.next).copied();
        if card.is_some() {
            self.next += 1;
        }
        card
    }
}

/// The nine paying hands, best first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hand {
    RoyalFlush,
    StraightFlush,
    FourOfAKind,
    FullHouse,
    Flush,
    Straight,
    ThreeOfAKind,
    TwoPair,
    JacksOrBetter,
    Nothing,
}

impl Hand {
    pub fn name(self) -> &'static str {
        match self {
            Hand::RoyalFlush => "ROYAL FLUSH",
            Hand::StraightFlush => "STRAIGHT FLUSH",
            Hand::FourOfAKind => "FOUR OF A KIND",
            Hand::FullHouse => "FULL HOUSE",
            Hand::Flush => "FLUSH",
            Hand::Straight => "STRAIGHT",
            Hand::ThreeOfAKind => "THREE OF A KIND",
            Hand::TwoPair => "TWO PAIR",
            Hand::JacksOrBetter => "JACKS OR BETTER",
            Hand::Nothing => "NO WIN",
        }
    }

    /// Payout per unit of the bet, from the 9/6 table.
    pub fn multiplier(self) -> u64 {
        match self {
            Hand::RoyalFlush => 250,
            Hand::StraightFlush => 50,
            Hand::FourOfAKind => 25,
            Hand::FullHouse => 9,
            Hand::Flush => 6,
            Hand::Straight => 4,
            Hand::ThreeOfAKind => 3,
            Hand::TwoPair => 2,
            Hand::JacksOrBetter => 1,
            Hand::Nothing => 0,
        }
    }

    /// Where the hand sits in the three-column paytable, if it pays.
    pub fn paytable_cell(self) -> Option<(usize, usize)> {
        match self {
            Hand::RoyalFlush => Some((0, 0)),
            Hand::StraightFlush => Some((0, 1)),
            Hand::FourOfAKind => Some((0, 2)),
            Hand::FullHouse => Some((1, 0)),
            Hand::Flush => Some((1, 1)),
            Hand::Straight => Some((1, 2)),
            Hand::ThreeOfAKind => Some((2, 0)),
            Hand::TwoPair => Some((2, 1)),
            Hand::JacksOrBetter => Some((2, 2)),
            Hand::Nothing => None,
        }
    }
}

/// The best five-card hand out of exactly five cards.
pub fn evaluate(cards: &[Card; 5]) -> Hand {
    let mut counts = [0u8; 15];
    for card in cards {
        counts[card.rank.value() as usize] += 1;
    }
    let flush = cards.iter().all(|card| card.suit == cards[0].suit);

    let mut ranks: Vec<u8> = cards.iter().map(|card| card.rank.value()).collect();
    ranks.sort_unstable();
    ranks.dedup();
    let straight = ranks.len() == 5
        && (ranks.windows(2).all(|pair| pair[1] == pair[0] + 1) || ranks == [2, 3, 4, 5, 14]);
    let royal = straight && flush && ranks.last() == Some(&14) && ranks[0] == 10;

    let four = counts.contains(&4);
    let three = counts.contains(&3);
    let pairs = counts.iter().filter(|&&count| count == 2).count();
    let jacks = (11..=14).any(|rank| counts[rank] == 2);

    if royal {
        Hand::RoyalFlush
    } else if straight && flush {
        Hand::StraightFlush
    } else if four {
        Hand::FourOfAKind
    } else if three && pairs == 1 {
        Hand::FullHouse
    } else if flush {
        Hand::Flush
    } else if straight {
        Hand::Straight
    } else if three {
        Hand::ThreeOfAKind
    } else if pairs == 2 {
        Hand::TwoPair
    } else if jacks {
        Hand::JacksOrBetter
    } else {
        Hand::Nothing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(rank: Rank, suit: Suit) -> Card {
        Card::new(rank, suit)
    }

    fn hand(cards: [(Rank, Suit); 5]) -> [Card; 5] {
        cards.map(|(rank, suit)| card(rank, suit))
    }

    #[test]
    fn recognizes_every_paying_hand() {
        assert_eq!(
            evaluate(&hand([
                (Rank::Ace, Suit::Spades),
                (Rank::King, Suit::Spades),
                (Rank::Queen, Suit::Spades),
                (Rank::Jack, Suit::Spades),
                (Rank::Ten, Suit::Spades),
            ])),
            Hand::RoyalFlush
        );
        assert_eq!(
            evaluate(&hand([
                (Rank::Nine, Suit::Hearts),
                (Rank::Eight, Suit::Hearts),
                (Rank::Seven, Suit::Hearts),
                (Rank::Six, Suit::Hearts),
                (Rank::Five, Suit::Hearts),
            ])),
            Hand::StraightFlush
        );
        assert_eq!(
            evaluate(&hand([
                (Rank::Four, Suit::Spades),
                (Rank::Four, Suit::Hearts),
                (Rank::Four, Suit::Diamonds),
                (Rank::Four, Suit::Clubs),
                (Rank::Nine, Suit::Spades),
            ])),
            Hand::FourOfAKind
        );
        assert_eq!(
            evaluate(&hand([
                (Rank::King, Suit::Spades),
                (Rank::King, Suit::Hearts),
                (Rank::King, Suit::Diamonds),
                (Rank::Two, Suit::Clubs),
                (Rank::Two, Suit::Spades),
            ])),
            Hand::FullHouse
        );
        assert_eq!(
            evaluate(&hand([
                (Rank::Two, Suit::Clubs),
                (Rank::Five, Suit::Clubs),
                (Rank::Eight, Suit::Clubs),
                (Rank::Jack, Suit::Clubs),
                (Rank::King, Suit::Clubs),
            ])),
            Hand::Flush
        );
        assert_eq!(
            evaluate(&hand([
                (Rank::Ace, Suit::Spades),
                (Rank::Two, Suit::Hearts),
                (Rank::Three, Suit::Diamonds),
                (Rank::Four, Suit::Clubs),
                (Rank::Five, Suit::Spades),
            ])),
            Hand::Straight
        );
        assert_eq!(
            evaluate(&hand([
                (Rank::Queen, Suit::Spades),
                (Rank::Queen, Suit::Hearts),
                (Rank::Queen, Suit::Diamonds),
                (Rank::Three, Suit::Clubs),
                (Rank::Seven, Suit::Spades),
            ])),
            Hand::ThreeOfAKind
        );
        assert_eq!(
            evaluate(&hand([
                (Rank::Jack, Suit::Spades),
                (Rank::Jack, Suit::Hearts),
                (Rank::Four, Suit::Diamonds),
                (Rank::Four, Suit::Clubs),
                (Rank::Nine, Suit::Spades),
            ])),
            Hand::TwoPair
        );
        assert_eq!(
            evaluate(&hand([
                (Rank::Ace, Suit::Spades),
                (Rank::Ace, Suit::Hearts),
                (Rank::Four, Suit::Diamonds),
                (Rank::Eight, Suit::Clubs),
                (Rank::Nine, Suit::Spades),
            ])),
            Hand::JacksOrBetter
        );
    }

    #[test]
    fn small_pairs_and_nothing_do_not_pay() {
        assert_eq!(
            evaluate(&hand([
                (Rank::Ten, Suit::Spades),
                (Rank::Ten, Suit::Hearts),
                (Rank::Four, Suit::Diamonds),
                (Rank::Eight, Suit::Clubs),
                (Rank::Nine, Suit::Spades),
            ])),
            Hand::Nothing
        );
        assert_eq!(
            evaluate(&hand([
                (Rank::Ace, Suit::Spades),
                (Rank::King, Suit::Hearts),
                (Rank::Queen, Suit::Diamonds),
                (Rank::Eight, Suit::Clubs),
                (Rank::Nine, Suit::Spades),
            ])),
            Hand::Nothing
        );
    }

    #[test]
    fn the_ace_can_be_low_or_high() {
        let low = hand([
            (Rank::Ace, Suit::Spades),
            (Rank::Two, Suit::Hearts),
            (Rank::Three, Suit::Diamonds),
            (Rank::Four, Suit::Clubs),
            (Rank::Five, Suit::Hearts),
        ]);
        assert_eq!(evaluate(&low), Hand::Straight);
        let high = hand([
            (Rank::Ace, Suit::Spades),
            (Rank::King, Suit::Spades),
            (Rank::Queen, Suit::Spades),
            (Rank::Jack, Suit::Spades),
            (Rank::Ten, Suit::Spades),
        ]);
        assert_eq!(evaluate(&high), Hand::RoyalFlush);
    }

    #[test]
    fn a_deck_holds_every_card_once() {
        let mut rng = Rng::new(7);
        let mut deck = Deck::shuffled(&mut rng);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..52 {
            assert!(seen.insert(deck.draw().unwrap()));
        }
        assert!(deck.draw().is_none());
        assert_eq!(seen.len(), 52);
    }

    #[test]
    fn a_seeded_rng_is_repeatable() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..20 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        assert!(Rng::new(1).below(1) == 0);
    }
}
