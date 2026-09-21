//! CF_DIB / CF_DIBV5 parsing to top-down straight-alpha BGRA, and DIBV5 encoding.

const BI_RGB: u32 = 0;
const BI_BITFIELDS: u32 = 3;

pub struct Decoded {
    pub width: u32,
    pub height: u32,
    /// top-down BGRA, straight alpha
    pub pixels: Vec<u8>,
    pub has_alpha: bool,
}

fn rd32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn rd16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

/// Parse a packed DIB (BITMAPINFOHEADER or BITMAPV5HEADER followed by color table and pixels).
pub fn decode(dib: &[u8]) -> Result<Decoded, &'static str> {
    if dib.len() < 40 {
        return Err("dib too small");
    }
    let hdr_size = rd32(dib, 0) as usize;
    if hdr_size < 40 || hdr_size > dib.len() {
        return Err("bad header size");
    }
    let width = rd32(dib, 4) as i32;
    let height_raw = rd32(dib, 8) as i32;
    let planes = rd16(dib, 12);
    let bpp = rd16(dib, 14) as u32;
    let compression = rd32(dib, 16);
    let clr_used = rd32(dib, 32) as usize;
    if planes != 1 || width <= 0 || height_raw == 0 {
        return Err("unsupported dib geometry");
    }
    let width = width as u32;
    let top_down = height_raw < 0;
    let height = height_raw.unsigned_abs();
    if width > 32768 || height > 32768 {
        return Err("dib too large");
    }

    // Masks (BI_BITFIELDS): for V4/V5 they're in the header; for BITMAPINFOHEADER they follow the header.
    let (mut rmask, mut gmask, mut bmask, mut amask) = (0u32, 0u32, 0u32, 0u32);
    let mut color_table_off = hdr_size;
    if compression == BI_BITFIELDS {
        if hdr_size >= 56 {
            rmask = rd32(dib, 40);
            gmask = rd32(dib, 44);
            bmask = rd32(dib, 48);
            amask = rd32(dib, 52);
        } else {
            if dib.len() < hdr_size + 12 {
                return Err("missing bitfields");
            }
            rmask = rd32(dib, hdr_size);
            gmask = rd32(dib, hdr_size + 4);
            bmask = rd32(dib, hdr_size + 8);
            color_table_off = hdr_size + 12;
        }
    } else if compression != BI_RGB {
        return Err("compressed dib unsupported");
    } else if hdr_size >= 56 && bpp == 32 {
        // V4/V5 with BI_RGB may still carry an alpha mask.
        amask = rd32(dib, 52);
    }

    let stride = ((width * bpp + 31) / 32 * 4) as usize;
    let palette_entries = if bpp <= 8 {
        if clr_used == 0 {
            1usize << bpp
        } else {
            clr_used
        }
    } else {
        0
    };
    let pixel_off = color_table_off + palette_entries * 4;
    let needed = pixel_off + stride * height as usize;
    if dib.len() < needed {
        return Err("dib pixel data truncated");
    }
    let palette = &dib[color_table_off..pixel_off];
    let src = &dib[pixel_off..];

    let mut out = vec![0u8; (width * height * 4) as usize];
    let mut any_alpha = false;
    let shift_of = |m: u32| -> (u32, u32) {
        if m == 0 {
            return (0, 0);
        }
        let s = m.trailing_zeros();
        let bits = (m >> s).trailing_ones();
        (s, bits)
    };
    let (rs, rb) = shift_of(rmask);
    let (gs, gb) = shift_of(gmask);
    let (bs, bb) = shift_of(bmask);
    let (as_, ab) = shift_of(amask);
    let expand = |v: u32, bits: u32| -> u8 {
        if bits == 0 {
            0
        } else if bits >= 8 {
            (v >> (bits - 8)) as u8
        } else {
            ((v * 255) / ((1 << bits) - 1)) as u8
        }
    };

    for y in 0..height as usize {
        let src_row = if top_down { y } else { height as usize - 1 - y };
        let row = &src[src_row * stride..src_row * stride + stride];
        let dst = &mut out[y * width as usize * 4..(y + 1) * width as usize * 4];
        match bpp {
            32 => {
                for x in 0..width as usize {
                    let px = rd32(row, x * 4);
                    let (b, g, r, a) = if compression == BI_BITFIELDS && rmask != 0 {
                        (
                            expand((px & bmask) >> bs, bb),
                            expand((px & gmask) >> gs, gb),
                            expand((px & rmask) >> rs, rb),
                            if amask != 0 { expand((px & amask) >> as_, ab) } else { 255 },
                        )
                    } else {
                        (row[x * 4], row[x * 4 + 1], row[x * 4 + 2], if amask != 0 || hdr_size < 56 { row[x * 4 + 3] } else { 255 })
                    };
                    if a != 255 {
                        any_alpha = true;
                    }
                    dst[x * 4] = b;
                    dst[x * 4 + 1] = g;
                    dst[x * 4 + 2] = r;
                    dst[x * 4 + 3] = a;
                }
            }
            24 => {
                for x in 0..width as usize {
                    dst[x * 4] = row[x * 3];
                    dst[x * 4 + 1] = row[x * 3 + 1];
                    dst[x * 4 + 2] = row[x * 3 + 2];
                    dst[x * 4 + 3] = 255;
                }
            }
            16 => {
                for x in 0..width as usize {
                    let px = rd16(row, x * 2) as u32;
                    let (r, g, b) = if compression == BI_BITFIELDS && rmask != 0 {
                        (expand((px & rmask) >> rs, rb), expand((px & gmask) >> gs, gb), expand((px & bmask) >> bs, bb))
                    } else {
                        (expand((px >> 10) & 0x1f, 5), expand((px >> 5) & 0x1f, 5), expand(px & 0x1f, 5))
                    };
                    dst[x * 4] = b;
                    dst[x * 4 + 1] = g;
                    dst[x * 4 + 2] = r;
                    dst[x * 4 + 3] = 255;
                }
            }
            8 | 4 | 1 => {
                for x in 0..width as usize {
                    let idx = match bpp {
                        8 => row[x] as usize,
                        4 => ((row[x / 2] >> (if x % 2 == 0 { 4 } else { 0 })) & 0xf) as usize,
                        _ => ((row[x / 8] >> (7 - (x % 8))) & 1) as usize,
                    };
                    let po = idx * 4;
                    if po + 3 < palette.len() {
                        dst[x * 4] = palette[po];
                        dst[x * 4 + 1] = palette[po + 1];
                        dst[x * 4 + 2] = palette[po + 2];
                    }
                    dst[x * 4 + 3] = 255;
                }
            }
            _ => return Err("unsupported bpp"),
        }
    }

    // A 32-bpp DIB whose alpha channel is all zero is almost always an opaque image with garbage alpha.
    let has_alpha = if bpp == 32 {
        let all_zero = out.chunks_exact(4).all(|p| p[3] == 0);
        if all_zero {
            for p in out.chunks_exact_mut(4) {
                p[3] = 255;
            }
            false
        } else {
            any_alpha
        }
    } else {
        false
    };
    Ok(Decoded { width, height, pixels: out, has_alpha })
}

/// Encode top-down straight-alpha BGRA as a packed CF_DIBV5 (BITMAPV5HEADER, BI_BITFIELDS, bottom-up).
pub fn encode_v5(width: u32, height: u32, bgra_top_down: &[u8]) -> Vec<u8> {
    let stride = (width * 4) as usize;
    let mut out = Vec::with_capacity(124 + stride * height as usize);
    let mut h = [0u8; 124];
    let put32 = |h: &mut [u8; 124], o: usize, v: u32| h[o..o + 4].copy_from_slice(&v.to_le_bytes());
    let put16 = |h: &mut [u8; 124], o: usize, v: u16| h[o..o + 2].copy_from_slice(&v.to_le_bytes());
    put32(&mut h, 0, 124);
    put32(&mut h, 4, width);
    put32(&mut h, 8, height); // positive = bottom-up
    put16(&mut h, 12, 1);
    put16(&mut h, 14, 32);
    put32(&mut h, 16, BI_BITFIELDS);
    put32(&mut h, 20, (stride * height as usize) as u32);
    put32(&mut h, 24, 2835);
    put32(&mut h, 28, 2835);
    put32(&mut h, 40, 0x00ff0000); // R
    put32(&mut h, 44, 0x0000ff00); // G
    put32(&mut h, 48, 0x000000ff); // B
    put32(&mut h, 52, 0xff000000); // A
    put32(&mut h, 56, 0x73524742); // LCS_sRGB 'sRGB'
    put32(&mut h, 108, 4); // LCS_GM_IMAGES
    out.extend_from_slice(&h);
    for y in (0..height as usize).rev() {
        out.extend_from_slice(&bgra_top_down[y * stride..(y + 1) * stride]);
    }
    out
}

/// Encode as a plain CF_DIB (BITMAPINFOHEADER, 32-bpp BI_RGB, bottom-up).
#[cfg(test)]
pub fn encode_v1(width: u32, height: u32, bgra_top_down: &[u8]) -> Vec<u8> {
    let stride = (width * 4) as usize;
    let mut out = Vec::with_capacity(40 + stride * height as usize);
    let mut h = [0u8; 40];
    h[0..4].copy_from_slice(&40u32.to_le_bytes());
    h[4..8].copy_from_slice(&width.to_le_bytes());
    h[8..12].copy_from_slice(&height.to_le_bytes());
    h[12..14].copy_from_slice(&1u16.to_le_bytes());
    h[14..16].copy_from_slice(&32u16.to_le_bytes());
    h[20..24].copy_from_slice(&((stride * height as usize) as u32).to_le_bytes());
    out.extend_from_slice(&h);
    for y in (0..height as usize).rev() {
        out.extend_from_slice(&bgra_top_down[y * stride..(y + 1) * stride]);
    }
    out
}

/// Read width/height from a PNG IHDR.
pub fn png_dimensions(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || &png[0..8] != b"\x89PNG\r\n\x1a\n" || &png[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
    let h = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
    Some((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v5_roundtrip() {
        let px: Vec<u8> = vec![1, 2, 3, 255, 4, 5, 6, 128, 7, 8, 9, 0, 10, 11, 12, 255];
        let dib = encode_v5(2, 2, &px);
        let d = decode(&dib).unwrap();
        assert_eq!(d.width, 2);
        assert_eq!(d.height, 2);
        assert_eq!(d.pixels, px);
        assert!(d.has_alpha);
    }

    #[test]
    fn v1_opaque_and_zero_alpha_fix() {
        let px: Vec<u8> = vec![1, 2, 3, 0, 4, 5, 6, 0];
        let dib = encode_v1(2, 1, &px);
        let d = decode(&dib).unwrap();
        assert!(!d.has_alpha);
        assert_eq!(d.pixels[3], 255);
        assert_eq!(d.pixels[4..7], [4, 5, 6]);
    }

    #[test]
    fn bpp24_bottom_up() {
        let mut dib = vec![0u8; 40];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&1u32.to_le_bytes());
        dib[8..12].copy_from_slice(&2u32.to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&24u16.to_le_bytes());
        // row stride for 1px @24bpp = 4 bytes; bottom row first
        dib.extend_from_slice(&[9, 9, 9, 0]);
        dib.extend_from_slice(&[1, 1, 1, 0]);
        let d = decode(&dib).unwrap();
        assert_eq!(&d.pixels[0..4], &[1, 1, 1, 255]);
        assert_eq!(&d.pixels[4..8], &[9, 9, 9, 255]);
    }

    #[test]
    fn png_dims() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(png_dimensions(&png), Some((640, 480)));
    }
}
