//! Files for the datasets tests to read.
//!
//! The workbook writer exists because this crate has no OOXML writer and does
//! not need one in production. It emits the smallest archive `calamine` will
//! open — the package relationships, the workbook part, its relationships, and
//! one worksheet per sheet with inline strings — so the reader is exercised
//! against a real zip rather than a mock.

use std::io::Write;
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;

/// Writes `contents` to `name` inside `directory` and returns the path.
pub fn write_file(directory: &Path, name: &str, contents: &[u8]) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, contents).expect("fixture file should be writable");
    path
}

/// One worksheet: its name and its grid of cells. An empty string is written
/// as an absent cell, which is what a spreadsheet actually stores.
pub struct Sheet<'a> {
    pub name: &'a str,
    pub rows: &'a [&'a [&'a str]],
}

/// Writes a workbook holding `sheets` and returns its path.
pub fn write_workbook(directory: &Path, name: &str, sheets: &[Sheet<'_>]) -> PathBuf {
    let path = directory.join(name);
    let file = std::fs::File::create(&path).expect("workbook should be writable");
    let mut archive = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default();

    let mut content_types = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>"#,
    );
    for index in 1..=sheets.len() {
        content_types.push_str(&format!(
            r#"<Override PartName="/xl/worksheets/sheet{index}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#
        ));
    }
    content_types.push_str("</Types>");
    add(&mut archive, options, "[Content_Types].xml", &content_types);

    add(
        &mut archive,
        options,
        "_rels/.rels",
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
    );

    let mut workbook = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>"#,
    );
    let mut relationships = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    for (position, sheet) in sheets.iter().enumerate() {
        let index = position + 1;
        workbook.push_str(&format!(
            r#"<sheet name="{}" sheetId="{index}" r:id="rId{index}"/>"#,
            escape(sheet.name)
        ));
        relationships.push_str(&format!(
            r#"<Relationship Id="rId{index}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{index}.xml"/>"#
        ));
        add(
            &mut archive,
            options,
            &format!("xl/worksheets/sheet{index}.xml"),
            &worksheet_xml(sheet.rows),
        );
    }
    workbook.push_str("</sheets></workbook>");
    relationships.push_str("</Relationships>");
    add(&mut archive, options, "xl/workbook.xml", &workbook);
    add(
        &mut archive,
        options,
        "xl/_rels/workbook.xml.rels",
        &relationships,
    );

    archive.finish().expect("workbook should close");
    path
}

/// A zip that is not a workbook: right signature, wrong contents.
pub fn write_plain_zip(directory: &Path, name: &str) -> PathBuf {
    let path = directory.join(name);
    let file = std::fs::File::create(&path).expect("archive should be writable");
    let mut archive = zip::ZipWriter::new(file);
    add(
        &mut archive,
        SimpleFileOptions::default(),
        "notes.txt",
        "not a workbook",
    );
    archive.finish().expect("archive should close");
    path
}

/// A compound document carrying the stream name a password-protected workbook
/// stores its contents under.
pub fn write_encrypted_workbook(directory: &Path, name: &str) -> PathBuf {
    let mut bytes: Vec<u8> = vec![0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];
    bytes.resize(512, 0);
    for byte in "EncryptedPackage".bytes() {
        bytes.push(byte);
        bytes.push(0);
    }
    write_file(directory, name, &bytes)
}

fn worksheet_xml(rows: &[&[&str]]) -> String {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
    );
    for (row_index, row) in rows.iter().enumerate() {
        let row_number = row_index + 1;
        xml.push_str(&format!(r#"<row r="{row_number}">"#));
        for (column_index, value) in row.iter().enumerate() {
            if value.is_empty() {
                continue;
            }
            let reference = format!("{}{row_number}", column_name(column_index));
            if value.parse::<f64>().is_ok() {
                xml.push_str(&format!(r#"<c r="{reference}"><v>{value}</v></c>"#));
            } else {
                xml.push_str(&format!(
                    r#"<c r="{reference}" t="inlineStr"><is><t>{}</t></is></c>"#,
                    escape(value)
                ));
            }
        }
        xml.push_str("</row>");
    }
    xml.push_str("</sheetData></worksheet>");
    xml
}

fn column_name(index: usize) -> String {
    let mut remaining = index;
    let mut name = Vec::new();
    loop {
        name.push(b'A' + (remaining % 26) as u8);
        if remaining < 26 {
            break;
        }
        remaining = remaining / 26 - 1;
    }
    name.reverse();
    String::from_utf8(name).expect("column name is ASCII")
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn add<W: Write + std::io::Seek>(
    archive: &mut zip::ZipWriter<W>,
    options: SimpleFileOptions,
    name: &str,
    contents: &str,
) {
    archive
        .start_file(name, options)
        .expect("archive entry should start");
    archive
        .write_all(contents.as_bytes())
        .expect("archive entry should write");
}
