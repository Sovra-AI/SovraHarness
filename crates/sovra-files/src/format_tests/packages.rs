//! Package parts are implementation details, not separate user documents.

use super::*;
use std::fmt::Write as _;

pub(crate) fn presentation() -> Vec<u8> {
    presentation_with_slides(120)
}

pub(crate) fn presentation_with_slides(slides: usize) -> Vec<u8> {
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut add = |name: &str, text: &str| {
        archive
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(text.as_bytes()).unwrap();
    };
    add(
        "[Content_Types].xml",
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>"#,
    );
    let mut presentation = String::from(
        r#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldIdLst>"#,
    );
    let mut relationships = String::from(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    for index in 1..=slides {
        write!(
            presentation,
            r#"<p:sldId id="{}" r:id="rId{index}"/>"#,
            255 + index
        )
        .unwrap();
        write!(relationships, r#"<Relationship Id="rId{index}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide{index}.xml"/>"#).unwrap();
        add(
            &format!("ppt/slides/slide{index}.xml"),
            &format!(
                r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>Package slide {index} marker{index}needle.</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#
            ),
        );
    }
    presentation.push_str("</p:sldIdLst></p:presentation>");
    relationships.push_str("</Relationships>");
    add("ppt/presentation.xml", &presentation);
    add("ppt/_rels/presentation.xml.rels", &relationships);
    archive.finish().unwrap().into_inner()
}

fn with_extra_parts(bytes: &[u8]) -> Vec<u8> {
    let mut source = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..source.len() {
        archive
            .raw_copy_file(source.by_index(index).unwrap())
            .unwrap();
    }
    for index in 0..120 {
        archive
            .start_file(
                format!("extras/part{index}.xml"),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        archive.write_all(b"<part/>").unwrap();
    }
    archive.finish().unwrap().into_inner()
}

#[test]
fn office_packages_over_one_hundred_parts_keep_document_identity_and_text() {
    for (bytes, expected_mime, expected_text) in [
        (
            presentation(),
            "presentationml.presentation",
            "marker120needle",
        ),
        (
            with_extra_parts(include_bytes!("../../tests/fixtures/invoice.docx")),
            "wordprocessingml.document",
            "93",
        ),
        (
            with_extra_parts(include_bytes!("../../tests/fixtures/inventory.xlsx")),
            "spreadsheetml.sheet",
            "Rose bushes",
        ),
    ] {
        // The package contents must win over a missing or misleading extension.
        let mime = detected_mime("misnamed.zip", &bytes);
        assert!(mime.ends_with(expected_mime), "{mime}");
        let content = prepare_bytes("package", "misnamed.zip", &bytes).unwrap();
        assert!(content.raw.is_none());
        assert!(
            content.text.contains(expected_text),
            "Missing {expected_text}"
        );
        assert!(!content.text.contains("File inside archive:"));
    }
}

#[test]
fn office_package_expansion_stays_bounded_after_removing_the_part_cutoff() {
    let mut bytes = presentation();
    let central = bytes
        .windows(4)
        .position(|part| part == b"PK\x01\x02")
        .unwrap();
    let declared = u32::try_from(MAX_DECODED_BYTES + 1).unwrap();
    bytes[central + 24..central + 28].copy_from_slice(&declared.to_le_bytes());
    assert!(detected_mime("misnamed.zip", &bytes).contains("presentationml"));
    assert!(prepare_bytes("over-budget", "misnamed.zip", &bytes).is_err());
}
