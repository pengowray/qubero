use super::*;

#[test]
fn unshuffling_puts_each_pixels_bytes_back_together() {
    // Two 16-bit pixels, 0x1234 and 0x5678, shuffled into their high bytes
    // then their low bytes.
    assert_eq!(unshuffle(&[0x12, 0x56, 0x34, 0x78], 2), vec![0x12, 0x34, 0x56, 0x78]);
}

/// The cards of a compressed image, as a header writes them.
pub(super) fn cards(lines: &[&str]) -> Cards {
    let mut b = Vec::new();
    for l in lines {
        let mut c = l.as_bytes().to_vec();
        c.resize(80, b' ');
        b.extend_from_slice(&c);
    }
    Cards::parse(&b)
}

#[test]
fn a_tile_at_the_far_edge_is_what_is_left() {
    let c = cards(&[
        "ZBITPIX =                   32",
        "ZNAXIS  =                    2",
        "ZNAXIS1 =                   50",
        "ZNAXIS2 =                   60",
        "ZTILE1  =                   20",
        "ZTILE2  =                   16",
        "ZCMPTYPE= 'RICE_1  '",
        "END",
    ]);
    let image = Image::from_cards(&c).unwrap();
    assert_eq!(image.tiles(), Some(12));
    assert_eq!(image.tile_box(0), (vec![0, 0], vec![20, 16]));
    assert_eq!(image.tile_box(2), (vec![40, 0], vec![10, 16]));
    assert_eq!(image.tile_box(11), (vec![40, 48], vec![10, 12]));
    assert_eq!((image.blocksize, image.bytepix), (32, 4));
}

/// A corrupt header can give axes that multiply to more pixels than a
/// `u64` counts, and so tiles, or a tile, of more than one counts. The
/// header is invalid, and the tile is still placed and described.
#[test]
fn an_image_of_more_pixels_than_a_u64_counts_is_invalid() {
    let square = |side: u64, tile: u64| {
        let lines = [
            "ZBITPIX =                    8".to_string(),
            "ZNAXIS  =                    2".into(),
            format!("ZNAXIS1 = {side:20}"),
            format!("ZNAXIS2 = {side:20}"),
            format!("ZTILE1  = {tile:20}"),
            format!("ZTILE2  = {tile:20}"),
            "ZCMPTYPE= 'NOCOMPRESS'".into(),
        ];
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        Image::from_cards(&cards(&refs)).unwrap()
    };
    let row = Row {
        place: Some(Place { stored: Stored::Compressed, count: 4, offset: 0, elem: b'B' }),
        has_data_column: true,
        zscale: None,
        zzero: None,
        zblank: None,
        quantized: false,
    };
    let invalid = "Not unpacked: the header is invalid. ZNAXISn say the image is 1,099,511,627,776 × 1,099,511,627,776 pixels, giving more than the maximum of 2^64.";
    // Tiles of one pixel, 2^80 of them.
    let t = decode(&square(1 << 40, 1), 5, &row, &[1, 2, 3, 4]);
    assert_eq!((t.tiles, t.pixel_count(), t.start, t.problem.as_deref()), (None, Some(1), vec![5, 0], Some(invalid)));
    assert!(t.pixels.is_empty());
    // One tile of 2^80 pixels, and a problem found after the header.
    let t = unread(&square(1 << 40, 1 << 40), 0, &row, 4, Some("past the heap".into()));
    assert_eq!((t.tiles, t.pixel_count(), t.problem.as_deref()), (Some(1), None, Some(invalid)));
    // A header that stays inside a u64, whose tile is over the limit.
    let t = decode(&square(1 << 31, 1 << 31), 0, &row, &[1, 2, 3, 4]);
    assert_eq!((t.tiles, t.pixel_count()), (Some(1), Some(1 << 62)));
    assert_eq!(t.problem.as_deref(), Some("Not unpacked: the tile is 4,611,686,018,427,387,904 pixels, over this viewer's limit of 16,777,216 pixels."));
    // None along an axis is no pixels, however many the others multiply to.
    assert_eq!(product(&[1 << 40, 1 << 40, 0]), Some(0));
}

/// A corrupt header can give a column's repeat that is more bytes than a
/// count holds. The column is as wide as a count goes, so its cell is past
/// the end of the row.
#[test]
fn a_column_too_wide_to_count_has_no_cell_in_the_row() {
    // A column whose repeat is more bytes than a count holds, of each
    // width, as the data column and as a column before it. And one whose
    // repeat is more than a count holds before it is multiplied at all.
    // Every four bytes of the row say 4, so a P descriptor is four bytes at
    // offset 4.
    let bytes: Vec<u8> = [0, 0, 0, 4].repeat(16);
    let place = |forms: &[String]| {
        let mut lines = vec!["ZBITPIX =                    8".to_string(), format!("TFIELDS = {:20}", forms.len())];
        for (n, form) in forms.iter().enumerate() {
            let name = if n + 1 == forms.len() { "COMPRESSED_DATA" } else { "ZSCALE" };
            lines.push(format!("TTYPE{}  = '{name}'", n + 1));
            lines.push(format!("TFORM{}  = '{form}'", n + 1));
        }
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        Image::from_cards(&cards(&refs)).unwrap().row(&bytes)
    };
    for (code, width) in [("I", 2), ("J", 4), ("K", 8), ("D", 8), ("M", 16), ("PB", 8), ("QB", 16)] {
        let repeat = usize::MAX / width + 1;
        let wide = place(&[format!("{repeat}{code}")]);
        assert_eq!((wide.place, wide.has_data_column), (None, true), "{repeat}{code}");
        let after = place(&[format!("{repeat}{code}"), "1PB".into()]);
        assert_eq!((after.place, after.zscale), (None, None), "{repeat}{code} before the data column");
    }
    let after = place(&[format!("{}L", usize::MAX), "1E".into(), "1PB".into()]);
    assert_eq!((after.place, after.zscale), (None, None));
    assert_eq!(place(&["99999999999999999999PB".into()]).place, None);
    assert_eq!(place(&["1PB".into()]).place, Some(Place { stored: Stored::Compressed, count: 4, offset: 4, elem: b'B' }));
}

/// A corrupt header can give a column a repeat of 0, so its cell has no
/// bytes. That is no cell: the row has no place in it, and its scale, zero
/// point and blank are the header's keywords.
#[test]
fn a_cell_too_short_for_one_element_is_no_cell() {
    // Every four bytes of the row say 4.
    let bytes: Vec<u8> = [0, 0, 0, 4].repeat(16);
    let row = |columns: &[(&str, &str)]| {
        let mut lines = vec![
            "ZBITPIX =                  -32".to_string(),
            "ZSCALE  =                  2.5".into(),
            "ZZERO   =                  1.5".into(),
            "ZBLANK  =                   -5".into(),
            format!("TFIELDS = {:20}", columns.len()),
        ];
        for (n, (name, form)) in columns.iter().enumerate() {
            lines.push(format!("TTYPE{}  = '{name}'", n + 1));
            lines.push(format!("TFORM{}  = '{form}'", n + 1));
        }
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        Image::from_cards(&cards(&refs)).unwrap().row(&bytes)
    };
    for form in ["0PB", "0QB"] {
        let r = row(&[("COMPRESSED_DATA", form)]);
        assert_eq!((r.place, r.has_data_column), (None, true), "{form}");
    }
    // An empty column takes no bytes, so the next one starts where it does.
    let r = row(&[("COMPRESSED_DATA", "0PB"), ("GZIP_COMPRESSED_DATA", "1PB")]);
    assert_eq!(r.place, Some(Place { stored: Stored::Gzip, count: 4, offset: 4, elem: b'B' }));
    for form in ["0D", "0E"] {
        let r = row(&[("ZSCALE", form), ("ZZERO", form)]);
        assert_eq!((r.zscale, r.zzero), (Some(2.5), Some(1.5)), "{form}");
    }
    for form in ["0J", "0K", "0I"] {
        assert_eq!(row(&[("ZBLANK", form)]).zblank, Some(-5), "{form}");
    }
    assert_eq!(row(&[("ZBLANK", "1J")]).zblank, Some(4));
}

#[test]
fn a_quoted_value_keeps_its_escaped_quote_and_loses_its_padding() {
    let c = cards(&["ZNAME1  = 'BLOCKSIZE'          / compression block size", "OBJECT  = 'it''s  '", "ZSCALE  =      2.5D-1 / real"]);
    assert_eq!(c.text("ZNAME1"), Some("BLOCKSIZE"));
    assert_eq!(c.text("OBJECT"), Some("it's"));
    assert_eq!(c.real("ZSCALE"), Some(0.25));
}

#[test]
fn an_algorithm_without_a_decoder_is_named() {
    let image = Image::from_cards(&cards(&["ZBITPIX =                   16", "ZNAXIS  =                    1", "ZNAXIS1 =                    4", "ZCMPTYPE= 'SQUASH_9'"])).unwrap();
    let row = Row {
        place: Some(Place { stored: Stored::Compressed, count: 3, offset: 0, elem: b'B' }),
        has_data_column: true,
        zscale: None,
        zzero: None,
        zblank: None,
        quantized: false,
    };
    let t = decode(&image, 0, &row, &[1, 2, 3]);
    assert!(t.pixels.is_empty());
    assert!(t.problem.unwrap().contains("SQUASH_9"));
}
