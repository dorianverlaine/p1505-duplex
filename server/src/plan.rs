//! Which source page goes on which side of which sheet.
//!
//! Sheets are numbered 1..=S in the order the front sides come out of the
//! printer. Sheet `i` carries page `2i-1` on the front and page `2i` on the
//! back; with an odd page count the last sheet gets a blank back so the stack
//! stays aligned when it goes back into the tray.

use std::fmt;
use std::str::FromStr;

/// One page of an output PDF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    /// 1-based page number in the source document.
    Source(u32),
    /// A blank page the size of the source's last page.
    Blank,
}

/// Order in which the back sides are sent, which depends on the paper path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Order {
    Forward,
    Reverse,
}

impl FromStr for Order {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "forward" => Ok(Order::Forward),
            "reverse" => Ok(Order::Reverse),
            _ => Err(format!("expected forward or reverse, got {s:?}")),
        }
    }
}

impl fmt::Display for Order {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Order::Forward => "forward",
            Order::Reverse => "reverse",
        })
    }
}

/// An inclusive range of sheets, 1-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sheets {
    pub first: u32,
    pub last: u32,
}

/// Number of sheets needed for `pages` pages printed on both sides.
pub fn sheet_count(pages: u32) -> u32 {
    pages.div_ceil(2)
}

impl Sheets {
    /// All sheets of a `pages`-page document.
    pub fn all(pages: u32) -> Sheets {
        Sheets {
            first: 1,
            last: sheet_count(pages),
        }
    }

    /// A user-chosen range, checked against the document.
    pub fn new(first: u32, last: u32, pages: u32) -> Result<Sheets, String> {
        let total = sheet_count(pages);
        if first < 1 || first > last || last > total {
            return Err(format!(
                "sheets must be within 1..={total}, got {first}..={last}"
            ));
        }
        Ok(Sheets { first, last })
    }

    fn iter(self) -> std::ops::RangeInclusive<u32> {
        self.first..=self.last
    }
}

/// Front sides of `sheets`, in the order they come out of the printer.
pub fn fronts(sheets: Sheets) -> Vec<Page> {
    sheets.iter().map(|i| Page::Source(2 * i - 1)).collect()
}

/// Back sides of `sheets` in sending order. Empty when the range has no
/// printable back (a single-page document), so no job needs to be sent.
pub fn backs(pages: u32, sheets: Sheets, order: Order) -> Vec<Page> {
    let back = |i: u32| {
        if 2 * i <= pages {
            Page::Source(2 * i)
        } else {
            Page::Blank
        }
    };
    let out: Vec<Page> = match order {
        Order::Forward => sheets.iter().map(back).collect(),
        Order::Reverse => sheets.iter().rev().map(back).collect(),
    };
    if out.iter().all(|p| *p == Page::Blank) {
        Vec::new()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Page::{Blank, Source as P};

    #[test]
    fn sheet_counts() {
        assert_eq!(sheet_count(1), 1);
        assert_eq!(sheet_count(2), 1);
        assert_eq!(sheet_count(5), 3);
        assert_eq!(sheet_count(6), 3);
    }

    #[test]
    fn odd_document_reverse() {
        let s = Sheets::all(5);
        assert_eq!(fronts(s), vec![P(1), P(3), P(5)]);
        // Last sheet has no page 6, so it gets a blank back, sent first.
        assert_eq!(backs(5, s, Order::Reverse), vec![Blank, P(4), P(2)]);
    }

    #[test]
    fn odd_document_forward() {
        assert_eq!(
            backs(5, Sheets::all(5), Order::Forward),
            vec![P(2), P(4), Blank]
        );
    }

    #[test]
    fn even_document() {
        let s = Sheets::all(4);
        assert_eq!(fronts(s), vec![P(1), P(3)]);
        assert_eq!(backs(4, s, Order::Reverse), vec![P(4), P(2)]);
        assert_eq!(backs(4, s, Order::Forward), vec![P(2), P(4)]);
    }

    #[test]
    fn single_page_has_no_back_job() {
        let s = Sheets::all(1);
        assert_eq!(fronts(s), vec![P(1)]);
        assert!(backs(1, s, Order::Reverse).is_empty());
    }

    #[test]
    fn two_pages() {
        let s = Sheets::all(2);
        assert_eq!(fronts(s), vec![P(1)]);
        assert_eq!(backs(2, s, Order::Reverse), vec![P(2)]);
    }

    #[test]
    fn reprint_part_of_the_backs() {
        // A jam after the blank back of sheet 3: sheets 1..=2 still need backs.
        let s = Sheets::new(1, 2, 5).unwrap();
        assert_eq!(backs(5, s, Order::Reverse), vec![P(4), P(2)]);
        // Only the last sheet: its back is blank, so nothing to send.
        assert!(backs(5, Sheets::new(3, 3, 5).unwrap(), Order::Reverse).is_empty());
    }

    #[test]
    fn reprint_part_of_the_fronts() {
        assert_eq!(fronts(Sheets::new(2, 3, 6).unwrap()), vec![P(3), P(5)]);
    }

    #[test]
    fn sheet_range_is_checked() {
        assert!(Sheets::new(0, 1, 5).is_err());
        assert!(Sheets::new(2, 1, 5).is_err());
        assert!(Sheets::new(1, 4, 5).is_err());
        assert!(Sheets::new(3, 3, 5).is_ok());
    }

    #[test]
    fn order_parses() {
        assert_eq!("reverse".parse::<Order>(), Ok(Order::Reverse));
        assert_eq!("forward".parse::<Order>(), Ok(Order::Forward));
        assert!("backwards".parse::<Order>().is_err());
    }
}
