//! Page selection on PDFs: pick, reorder and rotate pages, add blanks.
//!
//! The input is always the output of CUPS' pdftopdf, so it is well-formed;
//! lopdf is enough and no qpdf/Ghostscript is needed at run time.

use crate::plan::Page;
use anyhow::{Context, Result, anyhow, bail};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};

/// Attributes a page may inherit from its ancestors in the page tree.
const INHERITABLE: [&[u8]; 4] = [b"MediaBox", b"CropBox", b"Resources", b"Rotate"];

/// Catalog entries that point into the old page tree; dropped since the
/// output only goes to a printer.
const CATALOG_DROP: [&[u8]; 7] = [
    b"Outlines",
    b"Dests",
    b"StructTreeRoot",
    b"AcroForm",
    b"PageLabels",
    b"OpenAction",
    b"Names",
];

pub fn load(bytes: &[u8]) -> Result<Document> {
    Document::load_mem(bytes).context("reading PDF")
}

pub fn page_count(doc: &Document) -> u32 {
    doc.get_pages().len() as u32
}

/// A new PDF holding `pages` of `src` in that order, every page rotated by an
/// extra `rotate` degrees.
pub fn select(src: &Document, pages: &[Page], rotate: i64) -> Result<Vec<u8>> {
    let mut doc = src.clone();
    let source_pages = doc.get_pages();
    let last = *source_pages
        .values()
        .last()
        .ok_or_else(|| anyhow!("PDF has no pages"))?;

    let root = doc.new_object_id();
    let mut kids = Vec::with_capacity(pages.len());
    for page in pages {
        let mut dict = match page {
            Page::Source(n) => {
                let id = *source_pages
                    .get(n)
                    .ok_or_else(|| anyhow!("PDF has no page {n}"))?;
                resolved_page(&doc, id)?
            }
            Page::Blank => {
                let like = resolved_page(&doc, last)?;
                blank_page(&mut doc, &like)
            }
        };
        if rotate != 0 {
            let current = dict.get(b"Rotate").and_then(Object::as_i64).unwrap_or(0);
            dict.set("Rotate", (current + rotate).rem_euclid(360));
        }
        dict.set("Parent", root);
        kids.push(Object::Reference(doc.add_object(dict)));
    }
    let count = kids.len() as i64;
    doc.objects.insert(
        root,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }),
    );

    let catalog = doc.catalog_mut()?;
    catalog.set("Pages", root);
    for key in CATALOG_DROP {
        catalog.remove(key);
    }
    doc.prune_objects();
    doc.renumber_objects();
    doc.compress();

    let mut out = Vec::new();
    doc.save_to(&mut out).context("writing PDF")?;
    Ok(out)
}

/// A copy of page `id` with inherited attributes made explicit, so it can be
/// moved under a new, flat page tree.
fn resolved_page(doc: &Document, id: ObjectId) -> Result<Dictionary> {
    let mut dict = doc.get_dictionary(id)?.clone();
    let mut parent = dict.get(b"Parent").and_then(Object::as_reference).ok();
    let mut depth = 0;
    while let Some(pid) = parent {
        depth += 1;
        if depth > 64 {
            bail!("page tree too deep or cyclic");
        }
        let node = doc.get_dictionary(pid)?;
        for key in INHERITABLE {
            if !dict.has(key)
                && let Ok(v) = node.get(key)
            {
                dict.set(key, v.clone());
            }
        }
        parent = node.get(b"Parent").and_then(Object::as_reference).ok();
    }
    Ok(dict)
}

/// A blank page with the same media size as `like`.
fn blank_page(doc: &mut Document, like: &Dictionary) -> Dictionary {
    let a4 = || vec![0.into(), 0.into(), 595.into(), 842.into()];
    let media = like
        .get(b"MediaBox")
        .ok()
        .and_then(|o| doc.dereference(o).ok())
        .and_then(|(_, o)| o.as_array().ok().cloned())
        .unwrap_or_else(a4);
    let contents = doc.add_object(Stream::new(Dictionary::new(), Vec::new()));
    dictionary! {
        "Type" => "Page",
        "MediaBox" => media,
        "Resources" => Dictionary::new(),
        "Contents" => contents,
    }
}

/// A numbered A4 test document: a big page number in the middle and a
/// "TOP  page N" label along the top edge, for checking back-side order and
/// rotation.
pub fn test_document(pages: u32) -> Result<Vec<u8>> {
    let mut doc = Document::with_version("1.5");
    let root = doc.new_object_id();
    let font = |base: &str| {
        dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => base.to_string() }
    };
    let bold = doc.add_object(font("Helvetica-Bold"));
    let regular = doc.add_object(font("Helvetica"));
    let resources = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => bold, "F2" => regular },
    });

    let mut kids = Vec::new();
    for n in 1..=pages {
        let ops = format!(
            "BT /F1 200 Tf 200 400 Td ({n}) Tj ET\nBT /F2 30 Tf 60 780 Td (TOP  page {n}) Tj ET\n"
        );
        let contents = doc.add_object(Stream::new(Dictionary::new(), ops.into_bytes()));
        let page = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => root,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            "Resources" => resources,
            "Contents" => contents,
        });
        kids.push(Object::Reference(page));
    }
    doc.objects.insert(
        root,
        Object::Dictionary(
            dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => pages as i64 },
        ),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => root });
    doc.trailer.set("Root", catalog);
    doc.compress();

    let mut out = Vec::new();
    doc.save_to(&mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::Page::{Blank, Source as P};

    /// The page numbers printed on each page of `bytes` (None for blanks),
    /// and each page's /Rotate.
    fn describe(bytes: &[u8]) -> Vec<(Option<u32>, i64)> {
        let doc = load(bytes).unwrap();
        doc.get_pages()
            .values()
            .map(|&id| {
                let page = doc.get_dictionary(id).unwrap();
                let rotate = page.get(b"Rotate").and_then(Object::as_i64).unwrap_or(0);
                let text = String::from_utf8_lossy(&doc.get_page_content(id)).into_owned();
                let number = text
                    .split_once("TOP  page ")
                    .map(|(_, rest)| rest.split(')').next().unwrap().parse().unwrap());
                (number, rotate)
            })
            .collect()
    }

    #[test]
    fn test_document_has_numbered_pages() {
        let bytes = test_document(3).unwrap();
        assert_eq!(
            describe(&bytes),
            vec![(Some(1), 0), (Some(2), 0), (Some(3), 0)]
        );
    }

    #[test]
    fn selects_reorders_and_pads() {
        let src = load(&test_document(5).unwrap()).unwrap();
        let out = select(&src, &[Blank, P(4), P(2)], 0).unwrap();
        assert_eq!(describe(&out), vec![(None, 0), (Some(4), 0), (Some(2), 0)]);
        let fronts = select(&src, &[P(1), P(3), P(5)], 0).unwrap();
        assert_eq!(page_count(&load(&fronts).unwrap()), 3);
    }

    #[test]
    fn rotates_on_top_of_existing_rotation() {
        let src = load(&test_document(2).unwrap()).unwrap();
        let once = select(&src, &[P(2), P(1)], 180).unwrap();
        assert_eq!(describe(&once), vec![(Some(2), 180), (Some(1), 180)]);
        let twice = select(&load(&once).unwrap(), &[P(1)], 180).unwrap();
        assert_eq!(describe(&twice), vec![(Some(2), 0)]);
    }

    #[test]
    fn blank_page_takes_the_last_page_size() {
        let src = load(&test_document(1).unwrap()).unwrap();
        let out = load(&select(&src, &[Blank], 0).unwrap()).unwrap();
        let id = *out.get_pages().values().next().unwrap();
        let media = out
            .get_dictionary(id)
            .unwrap()
            .get(b"MediaBox")
            .unwrap()
            .as_array()
            .unwrap()
            .clone();
        let media: Vec<i64> = media.iter().map(|o| o.as_i64().unwrap()).collect();
        assert_eq!(media, vec![0, 0, 595, 842]);
    }

    #[test]
    fn inherited_attributes_survive_flattening() {
        // Put MediaBox and Rotate on the Pages node only.
        let mut doc = load(&test_document(2).unwrap()).unwrap();
        let pages_id = doc
            .catalog()
            .unwrap()
            .get(b"Pages")
            .unwrap()
            .as_reference()
            .unwrap();
        for id in doc.get_pages().into_values() {
            doc.get_dictionary_mut(id).unwrap().remove(b"MediaBox");
        }
        let node = doc.get_dictionary_mut(pages_id).unwrap();
        node.set("MediaBox", vec![0.into(), 0.into(), 612.into(), 792.into()]);
        node.set("Rotate", 90);

        let out = load(&select(&doc, &[P(2)], 180).unwrap()).unwrap();
        let id = *out.get_pages().values().next().unwrap();
        let page = out.get_dictionary(id).unwrap();
        assert_eq!(page.get(b"Rotate").unwrap().as_i64().unwrap(), 270);
        assert_eq!(
            page.get(b"MediaBox").unwrap().as_array().unwrap()[2]
                .as_i64()
                .unwrap(),
            612
        );
    }

    #[test]
    fn missing_page_is_an_error() {
        let src = load(&test_document(2).unwrap()).unwrap();
        assert!(select(&src, &[P(3)], 0).is_err());
    }
}
