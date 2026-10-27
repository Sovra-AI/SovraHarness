//! Small synthetic format fixtures prove the real codec integrations, offline.

use super::*;

#[test]
fn source_and_text_content_is_read_without_an_extension_allowlist() {
    let source = "// Rust source — 你好\nfn main() { let total = 93; }\n";
    for name in [
        "main.rs",
        "index.ts",
        "view.tsx",
        "app.jsx",
        "script.js",
        "main.c",
        "main.cpp",
        "header.h",
        "app.cs",
        "Main.java",
        "main.go",
        "script.py",
        "script.rb",
        "app.swift",
        "build.sh",
        "build.bat",
        "build.ps1",
        "query.sql",
        "config.toml",
        "config.yaml",
        "config.ini",
        "settings.cfg",
        "style.css",
        "page.scss",
        "build.gradle",
        "Makefile",
        "Dockerfile",
        ".gitignore",
        ".env",
        "README",
        "notes.log",
        "notes.txt",
        "notes.xyzzy",
        "drawing.dxf",
        "mesh.obj",
        "mesh.stl",
        "program.asm",
        "schema.proto",
        "module.wat",
        "MAIN.RS",
        "no-extension",
        "picture.png",
    ] {
        let file = prepare_bytes("source", name, source.as_bytes())
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(file.text, source, "{name} must preserve the full source");
        assert_eq!(file.attachment.purpose, ModelPurpose::Documents, "{name}");
    }
}

#[test]
fn reads_office_documents_spreadsheets_email_and_nested_zip_members() {
    for (name, bytes, expected) in [
        (
            "invoice.docx",
            include_bytes!("../tests/fixtures/invoice.docx").as_slice(),
            "93",
        ),
        (
            "inventory.xlsx",
            include_bytes!("../tests/fixtures/inventory.xlsx").as_slice(),
            "Rose bushes",
        ),
        (
            "message.eml",
            include_bytes!("../tests/fixtures/message.eml").as_slice(),
            "Tuesday",
        ),
        (
            "bundle.zip",
            include_bytes!("../tests/fixtures/bundle.zip").as_slice(),
            "blue umbrella",
        ),
    ] {
        let content = prepare_bytes("format-test", name, bytes).unwrap();
        assert!(
            content.text.contains(expected),
            "Missing {expected} from {name}: {}",
            content.text
        );
    }
}

#[test]
fn archive_expansion_limit_rejects_a_small_compressed_bomb() {
    let bytes = include_bytes!("../tests/fixtures/oversized.zip");
    assert!(bytes.len() < 100_000);
    assert!(prepare_bytes("bomb", "large.zip", bytes).is_err());
}

fn pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in offsets.iter().skip(1) {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF",
            offsets.len()
        )
        .as_bytes(),
    );
    bytes
}

fn stream(data: &[u8], attributes: &str) -> Vec<u8> {
    let mut bytes = format!("<< /Length {} {attributes} >>\nstream\n", data.len()).into_bytes();
    bytes.extend_from_slice(data);
    bytes.extend_from_slice(b"\nendstream");
    bytes
}

#[test]
fn extracts_pdf_text_and_turns_scanned_pages_into_real_picture_inputs() {
    let text_pdf = pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        stream(b"BT /F1 12 Tf 20 100 Td (Garden invoice total: 93 dirhams. Delivery is Tuesday.) Tj ET", ""),
    ]);
    let content = prepare_bytes("pdf-text", "invoice.pdf", &text_pdf).unwrap();
    assert!(content.text.contains("93"));
    assert_eq!(content.attachment.purpose, ModelPurpose::Documents);

    let mut jpeg = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(32, 24)
        .write_to(&mut jpeg, ImageFormat::Jpeg)
        .unwrap();
    let scan = pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 32 24] /Resources << /XObject << /Im1 4 0 R >> >> /Contents 5 0 R >>".to_vec(),
        stream(jpeg.get_ref(), "/Type /XObject /Subtype /Image /Width 32 /Height 24 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode"),
        stream(b"q 32 0 0 24 0 0 cm /Im1 Do Q", ""),
    ]);
    let content = prepare_bytes("pdf-scan", "scan.pdf", &scan).unwrap();
    assert_eq!(content.attachment.purpose, ModelPurpose::Images);
    assert_eq!(content.media.len(), 1);
    assert_eq!(content.media[0].mime_type, "image/png");
}

#[test]
fn containers_are_decoded_by_contents_when_renamed_or_missing_extensions() {
    for (bytes, expected) in [
        (
            include_bytes!("../tests/fixtures/invoice.docx").as_slice(),
            "93",
        ),
        (
            include_bytes!("../tests/fixtures/inventory.xlsx").as_slice(),
            "Rose bushes",
        ),
        (
            include_bytes!("../tests/fixtures/bundle.zip").as_slice(),
            "blue umbrella",
        ),
    ] {
        for name in ["no-extension", "misleading.rs", "misleading.jpg"] {
            let file = prepare_bytes("renamed", name, bytes).unwrap();
            assert!(file.raw.is_none(), "{name}: {}", file.text);
            assert!(file.text.contains(expected), "{name}: {}", file.text);
        }
    }
}

#[test]
fn actual_rust_source_and_unknown_bytes_survive_managed_storage() {
    let root = std::env::temp_dir().join(format!("sovra-all-formats-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let store = LocalFileStore::new(root.join("managed"));
    let source = include_bytes!("lib.rs");
    for (id, name, bytes) in [
        ("rust", "lib.rs", source.as_slice()),
        (
            "binary",
            "unknown.custom",
            b"\0\xffprivate data 93\0".as_slice(),
        ),
        ("empty", "empty.weird", b"".as_slice()),
    ] {
        let path = root.join(name);
        fs::write(&path, bytes).unwrap();
        store.prepare(id, &path).unwrap();
        fs::remove_file(&path).unwrap();
        let restored = store.load(id).unwrap();
        if id == "rust" {
            assert_eq!(restored.text.as_bytes(), bytes);
        } else {
            assert_eq!(restored.raw.as_deref(), Some(bytes));
            assert!(restored.text.contains("CONTENT NOT FULLY DECODED"));
        }
        store.discard(id).unwrap();
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn whitespace_and_empty_archives_are_still_valid_attachments() {
    assert_eq!(
        prepare_bytes("spaces", "file.rs", b"\n\t \r\n")
            .unwrap()
            .text,
        "\n\t \r\n"
    );
    let empty = zip::ZipWriter::new(Cursor::new(Vec::new()))
        .finish()
        .unwrap()
        .into_inner();
    let file = prepare_bytes("empty-zip", "empty.zip", &empty).unwrap();
    assert!(
        file.text.contains("empty") || file.text.contains("no regular"),
        "{}",
        file.text
    );
}

#[test]
fn renders_svg_pixels_and_excludes_external_resources() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="#ff0000"/><image href="file:///etc/passwd" width="20" height="10"/></svg>"##;
    let content = prepare_bytes("svg", "diagram.svg", svg).unwrap();
    assert_eq!(content.attachment.purpose, ModelPurpose::Images);
    let pixels = image::load_from_memory(&content.media[0].data)
        .unwrap()
        .into_rgba8();
    assert_eq!(pixels.dimensions(), (20, 10));
    assert_eq!(pixels.get_pixel(5, 5).0, [255, 0, 0, 255]);
}

#[test]
fn compressed_sources_are_read_as_text_including_typescript() {
    for (name, bytes) in [
        (
            "source.ts.gz",
            include_bytes!("../tests/fixtures/source.ts.gz").as_slice(),
        ),
        (
            "source.rs.xz",
            include_bytes!("../tests/fixtures/source.rs.xz").as_slice(),
        ),
        (
            "source.rs.lzma",
            include_bytes!("../tests/fixtures/source.rs.lzma").as_slice(),
        ),
        (
            "source.rs.bz2",
            include_bytes!("../tests/fixtures/source.rs.bz2").as_slice(),
        ),
    ] {
        let content = prepare_bytes("compressed", name, bytes).unwrap();
        assert!(
            content.text.contains("invoice total = 93"),
            "{name}: {}",
            content.text
        );
    }
}

fn zstd_frame(text: &[u8]) -> Vec<u8> {
    let mut frame = vec![
        0x28,
        0xb5,
        0x2f,
        0xfd,
        0x20,
        u8::try_from(text.len()).unwrap(),
    ];
    let header = (u32::try_from(text.len()).unwrap() << 3) | 1;
    frame.extend_from_slice(&header.to_le_bytes()[..3]);
    frame.extend_from_slice(text);
    frame
}

#[test]
fn zstandard_reads_all_frames_and_skips_metadata_frames() {
    let mut bytes = zstd_frame(b"First invoice: 93\n");
    bytes.extend_from_slice(&[0x50, 0x2a, 0x4d, 0x18, 4, 0, 0, 0, 1, 2, 3, 4]);
    bytes.extend(zstd_frame(b"Second invoice: 71\n"));
    let content = prepare_bytes("zstd", "invoices.txt.zst", &bytes).unwrap();
    assert!(content.text.contains("First invoice: 93"));
    assert!(
        content.text.contains("Second invoice: 71"),
        "{}",
        content.text
    );
}

#[test]
fn notebook_code_and_output_are_read_without_running_cells() {
    let bytes = br#"{"nbformat":4,"nbformat_minor":5,"metadata":{},"cells":[{"cell_type":"code","metadata":{},"source":["total = 93\n"],"execution_count":1,"outputs":[{"output_type":"stream","name":"stdout","text":["result is 93\n"]}]}]}"#;
    let file = prepare_bytes("notebook", "analysis.ipynb", bytes).unwrap();
    assert!(file.text.contains("total = 93"), "{}", file.text);
    assert!(file.text.contains("result is 93"), "{}", file.text);
}

#[test]
fn zip_mimetype_headers_cannot_allocate_the_declared_untrusted_size() {
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    archive
        .start_file(
            "mimetype",
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )
        .unwrap();
    archive.write_all(b"application/epub+zip").unwrap();
    let mut bytes = archive.finish().unwrap().into_inner();
    let central = bytes
        .windows(4)
        .position(|part| part == b"PK\x01\x02")
        .unwrap();
    bytes[central + 20..central + 28].fill(0xfe);
    let mime = detected_mime("hostile.zip", &bytes);
    assert_eq!(mime, "application/zip");
}

#[test]
fn hwpx_preserves_xml_members_using_the_bounded_archive_reader() {
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in [
        ("mimetype", "application/hwp+zip"),
        (
            "Contents/section0.xml",
            "<hp:section xmlns:hp=\"urn:test\"><hp:t>Invoice total 93</hp:t></hp:section>",
        ),
    ] {
        archive
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(bytes.as_bytes()).unwrap();
    }
    let bytes = archive.finish().unwrap().into_inner();
    let file = prepare_bytes("hwpx", "document.hwpx", &bytes).unwrap();
    assert!(file.text.contains("Invoice total 93"), "{}", file.text);
    assert!(file.text.contains("not rendered page layout"));
}
