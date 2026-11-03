//! A one-column scrollbar, shared by the shell and by programs that read long
//! things. It is deliberately plain data: the caller owns the page and chooses
//! the palette slots.

use crate::screen::{Cell, Screen};

/// Where a scrollable view is: first visible row, visible rows and total rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scroll {
    pub first: usize,
    pub visible: usize,
    pub total: usize,
}

/// Draw a track and a proportional thumb at `x`, from `top` to `bottom`
/// (inclusive). Nothing is drawn when the content fits, so a short chapter has
/// no furniture.
// The explicit colours keep this usable by both the shell's theme and a
// program that only receives the page/ink slots.
#[allow(clippy::too_many_arguments)]
pub fn draw_scrollbar(
    screen: &mut Screen,
    x: u32,
    top: u32,
    bottom: u32,
    scroll: Scroll,
    track: u8,
    thumb: u8,
    background: u8,
) {
    let Scroll {
        first,
        visible,
        total,
    } = scroll;
    if total <= visible || visible == 0 || bottom < top {
        return;
    }
    let height = (bottom - top + 1) as usize;
    let thumb_height = ((visible * height) / total).max(1).min(height);
    let travel = height - thumb_height;
    let max_first = total - visible;
    let offset = first.min(max_first) * travel / max_first;
    for row in 0..height {
        let in_thumb = row >= offset && row < offset + thumb_height;
        let (ch, colour) = if in_thumb {
            ('\u{2588}', thumb)
        } else {
            ('\u{2502}', track)
        };
        screen.put(x, top + row as u32, Cell::new(ch, colour, background));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scroll(first: usize, total: usize) -> Scroll {
        Scroll {
            first,
            visible: 3,
            total,
        }
    }

    #[test]
    fn the_thumb_follows_the_view() {
        let mut screen = Screen::new(20, 6);
        draw_scrollbar(&mut screen, 19, 0, 5, scroll(0, 30), 8, 11, 0);
        assert_eq!(screen.get(19, 0).unwrap().ch, '\u{2588}');
        assert_ne!(screen.get(19, 5).unwrap().ch, '\u{2588}');

        draw_scrollbar(&mut screen, 19, 0, 5, scroll(27, 30), 8, 11, 0);
        assert_eq!(screen.get(19, 5).unwrap().ch, '\u{2588}');
    }

    #[test]
    fn a_short_view_draws_nothing() {
        let mut screen = Screen::new(20, 6);
        draw_scrollbar(&mut screen, 19, 0, 5, scroll(0, 3), 8, 11, 0);
        assert_eq!(screen.get(19, 0).unwrap().ch, ' ');
    }
}
