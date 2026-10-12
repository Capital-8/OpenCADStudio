//! Colour books: AutoCAD `.acb` files.
//!
//! An `.acb` is XML: a `bookName` and `colorPage`s of `colorEntry`s, each a
//! `colorName` and an `RGB8` value. Books from some vendors encrypt their
//! values; those cannot be read.

/// One named colour.
#[derive(Debug, Clone, PartialEq)]
pub struct BookColor {
    pub name: String,
    pub rgb: (u8, u8, u8),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColorBook {
    pub name: String,
    /// Every colour, page after page.
    pub colors: Vec<BookColor>,
    /// Where each page starts in `colors`.
    page_starts: Vec<usize>,
}

/// Pages for a book without `colorPage`s, as AutoCAD shows them.
const PAGE_SIZE: usize = 7;

impl ColorBook {
    pub fn page_count(&self) -> usize {
        self.page_starts.len()
    }

    /// The colours of one page, and the index of its first colour.
    pub fn page(&self, page: usize) -> (usize, &[BookColor]) {
        let start = self.page_starts.get(page).copied().unwrap_or(0);
        let end = self
            .page_starts
            .get(page + 1)
            .copied()
            .unwrap_or(self.colors.len());
        (start, &self.colors[start..end])
    }

    /// The page holding colour `index`.
    pub fn page_of(&self, index: usize) -> usize {
        self.page_starts
            .partition_point(|&start| start <= index)
            .saturating_sub(1)
    }

    /// The colour with this name, as stored after the `$` of `BOOK$NAME`.
    pub fn find(&self, name: &str) -> Option<usize> {
        self.colors.iter().position(|c| c.name == name)
    }

    /// The `BOOK$NAME` identity objects and layers store.
    pub fn identity(&self, index: usize) -> String {
        format!("{}${}", self.name, self.colors[index].name)
    }
}

/// Read an `.acb` file's text.
pub fn parse(text: &str) -> Result<ColorBook, String> {
    let doc = roxmltree::Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|e| e.to_string())?;
    let root = doc.root_element();
    if !root.has_tag_name("colorBook") {
        return Err(crate::t!("Not a color book").into_owned());
    }
    let mut book = ColorBook {
        name: text_of(root, "bookName").unwrap_or_default().to_string(),
        colors: Vec::new(),
        page_starts: Vec::new(),
    };
    let mut encrypted = false;
    let mut read_entries = |parent: roxmltree::Node<'_, '_>, book: &mut ColorBook| {
        for entry in parent.children().filter(|n| n.has_tag_name("colorEntry")) {
            let rgb = child(entry, "RGB8").and_then(|rgb| {
                let channel = |tag| text_of(rgb, tag)?.parse::<u8>().ok();
                Some((channel("red")?, channel("green")?, channel("blue")?))
            });
            match (text_of(entry, "colorName"), rgb) {
                (Some(name), Some(rgb)) => book.colors.push(BookColor {
                    name: name.to_string(),
                    rgb,
                }),
                _ => encrypted = true,
            }
        }
    };
    let mut pages = root
        .children()
        .filter(|n| n.has_tag_name("colorPage"))
        .peekable();
    if pages.peek().is_some() {
        for page in pages {
            let start = book.colors.len();
            read_entries(page, &mut book);
            if book.colors.len() > start {
                book.page_starts.push(start);
            }
        }
    } else {
        read_entries(root, &mut book);
        book.page_starts = (0..book.colors.len()).step_by(PAGE_SIZE).collect();
    }
    if book.colors.is_empty() {
        return Err(if encrypted {
            crate::t!("This color book is encrypted and cannot be read")
        } else {
            crate::t!("The color book has no colors")
        }
        .into_owned());
    }
    Ok(book)
}

fn child<'a, 'i>(node: roxmltree::Node<'a, 'i>, tag: &str) -> Option<roxmltree::Node<'a, 'i>> {
    node.children().find(|n| n.has_tag_name(tag))
}

fn text_of<'a>(node: roxmltree::Node<'a, '_>, tag: &str) -> Option<&'a str> {
    child(node, tag).and_then(|n| n.text()).map(str::trim)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, (r, g, b): (u8, u8, u8)) -> String {
        format!(
            "<colorEntry><colorName>{name}</colorName>\
             <RGB8><red>{r}</red><green>{g}</green><blue>{b}</blue></RGB8></colorEntry>"
        )
    }

    #[test]
    fn reads_pages_and_colours() {
        let text = format!(
            "\u{feff}<?xml version=\"1.0\"?><colorBook><bookName>RAL CLASSIC</bookName>\
             <colorPage><pageColor><RGB8><red>1</red><green>2</green><blue>3</blue></RGB8></pageColor>{}{}</colorPage>\
             <colorPage>{}</colorPage></colorBook>",
            entry("RAL 1000", (190, 189, 127)),
            entry("RAL 1001", (194, 176, 120)),
            entry("RAL 5015", (34, 113, 179)),
        );
        let book = parse(&text).unwrap();
        assert_eq!(book.name, "RAL CLASSIC");
        assert_eq!(book.page_count(), 2);
        assert_eq!(book.page(1), (2, &book.colors[2..]));
        assert_eq!(book.page_of(1), 0);
        assert_eq!(book.page_of(2), 1);
        let blue = book.find("RAL 5015").unwrap();
        assert_eq!(book.colors[blue].rgb, (34, 113, 179));
        assert_eq!(book.identity(blue), "RAL CLASSIC$RAL 5015");
    }

    #[test]
    fn a_book_without_pages_gets_pages_of_seven() {
        let entries: String = (0..9).map(|i| entry(&format!("C{i}"), (i, i, i))).collect();
        let book = parse(&format!(
            "<colorBook><bookName>B</bookName>{entries}</colorBook>"
        ))
        .unwrap();
        assert_eq!(book.page_count(), 2);
        assert_eq!(book.page(1).1.len(), 2);
    }

    #[test]
    fn rejects_unreadable_books() {
        assert!(parse("not xml").is_err());
        assert!(parse("<palette/>").is_err());
        let encrypted = "<colorBook><bookName>P</bookName><colorPage><colorEntry>\
                         <colorName>X</colorName><RGB8Encrypt>AbC</RGB8Encrypt></colorEntry>\
                         </colorPage></colorBook>";
        assert!(parse(encrypted).is_err());
    }
}
