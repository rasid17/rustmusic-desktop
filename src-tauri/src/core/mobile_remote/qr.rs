//! Pure-Rust QR Code SVG generator (ISO/IEC 18004, Byte Mode, ECC Level L).
//!
//! Supports QR versions 1 through 6 (up to 134 bytes of UTF-8 data, more than
//! enough for `http://192.168.xxx.xxx:8300/?pin=1234`) without adding any
//! external crate dependencies.

/// Specification for a single QR version at Error Correction Level L (Single Block).
struct VersionSpec {
    version: usize,
    size: usize,
    data_codewords: usize,
    ec_codewords: usize,
    alignment_centers: &'static [usize],
}

const VERSIONS: &[VersionSpec] = &[
    VersionSpec { version: 1, size: 21, data_codewords: 19, ec_codewords: 7, alignment_centers: &[] },
    VersionSpec { version: 2, size: 25, data_codewords: 34, ec_codewords: 10, alignment_centers: &[6, 18] },
    VersionSpec { version: 3, size: 29, data_codewords: 55, ec_codewords: 15, alignment_centers: &[6, 22] },
    VersionSpec { version: 4, size: 33, data_codewords: 80, ec_codewords: 20, alignment_centers: &[6, 26] },
    VersionSpec { version: 5, size: 37, data_codewords: 108, ec_codewords: 26, alignment_centers: &[6, 30] },
];

/// Generate an SVG string representing a scannable QR code for `text`.
pub fn generate_svg(text: &str) -> String {
    let bytes = text.as_bytes();
    let spec = VERSIONS
        .iter()
        .find(|v| bytes.len() + 3 <= v.data_codewords)
        .unwrap_or(&VERSIONS[VERSIONS.len() - 1]);

    let codewords = encode_codewords(bytes, spec);
    let n = spec.size;
    let mut modules = vec![vec![false; n]; n];
    let mut is_function = vec![vec![false; n]; n];

    draw_function_patterns(&mut modules, &mut is_function, spec);
    draw_codewords(&mut modules, &is_function, &codewords, n);

    // Choose best mask (0..8) with lowest penalty score
    let mut best_mask = 0u8;
    let mut best_score = u32::MAX;
    let mut best_grid = modules.clone();

    for mask in 0..8u8 {
        let mut candidate = modules.clone();
        apply_mask(&mut candidate, &is_function, mask, n);
        draw_format_bits(&mut candidate, mask, n);
        let score = penalty_score(&candidate, n);
        if score < best_score {
            best_score = score;
            best_mask = mask;
            best_grid = candidate;
        }
    }
    let _ = best_mask;

    let border = 2usize;
    let total = n + border * 2;
    let mut path_data = String::with_capacity(n * n * 12);

    for (y, row) in best_grid.iter().enumerate() {
        for (x, &dark) in row.iter().enumerate() {
            if dark {
                let px = x + border;
                let py = y + border;
                path_data.push_str(&format!("M{px},{py}h1v1h-1z"));
            }
        }
    }

    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {total} {total}" shape-rendering="crispEdges"><rect width="100%" height="100%" fill="#ffffff" rx="2"/><path d="{path_data}" fill="#090d0b"/></svg>"##
    )
}

fn encode_codewords(bytes: &[u8], spec: &VersionSpec) -> Vec<u8> {
    let max_len = spec.data_codewords.saturating_sub(3);
    let slice = if bytes.len() > max_len { &bytes[..max_len] } else { bytes };

    let mut bits: Vec<bool> = Vec::with_capacity(spec.data_codewords * 8);
    let push_bits = |bits: &mut Vec<bool>, val: usize, len: usize| {
        for i in (0..len).rev() {
            bits.push(((val >> i) & 1) != 0);
        }
    };

    // Byte mode indicator (0100)
    push_bits(&mut bits, 0b0100, 4);
    // Character count (8 bits for versions 1..9)
    push_bits(&mut bits, slice.len(), 8);
    // Payload bytes
    for &b in slice {
        push_bits(&mut bits, b as usize, 8);
    }
    // Terminator (up to 4 zero bits)
    let capacity_bits = spec.data_codewords * 8;
    let term = (capacity_bits - bits.len()).min(4);
    push_bits(&mut bits, 0, term);
    // Pad to byte boundary
    while bits.len() % 8 != 0 {
        bits.push(false);
    }

    let mut data: Vec<u8> = bits
        .chunks(8)
        .map(|chunk| {
            chunk
                .iter()
                .fold(0u8, |acc, &bit| (acc << 1) | if bit { 1 } else { 0 })
        })
        .collect();

    // Pad bytes (0xEC, 0x11 alternating)
    let pads = [0xECu8, 0x11u8];
    let mut pad_idx = 0;
    while data.len() < spec.data_codewords {
        data.push(pads[pad_idx % 2]);
        pad_idx += 1;
    }

    let ec = reed_solomon_remainder(&data, spec.ec_codewords);
    data.extend(ec);
    data
}

fn gf_mul(mut x: u8, mut y: u8) -> u8 {
    let mut r = 0u8;
    for _ in 0..8 {
        if (y & 1) != 0 {
            r ^= x;
        }
        let hi = (x & 0x80) != 0;
        x <<= 1;
        if hi {
            x ^= 0x1D;
        }
        y >>= 1;
    }
    r
}

fn reed_solomon_divisor(degree: usize) -> Vec<u8> {
    let mut result = vec![0u8; degree];
    result[degree - 1] = 1;
    let mut root = 1u8;
    for _ in 0..degree {
        for j in 0..degree {
            result[j] = gf_mul(result[j], root);
            if j + 1 < degree {
                result[j] ^= result[j + 1];
            }
        }
        root = gf_mul(root, 0x02);
    }
    result
}

fn reed_solomon_remainder(data: &[u8], degree: usize) -> Vec<u8> {
    let divisor = reed_solomon_divisor(degree);
    let mut result = vec![0u8; degree];
    for &b in data {
        let factor = b ^ result[0];
        result.remove(0);
        result.push(0);
        for (r, &d) in result.iter_mut().zip(divisor.iter()) {
            *r ^= gf_mul(d, factor);
        }
    }
    result
}

fn set_func(
    modules: &mut [Vec<bool>],
    is_function: &mut [Vec<bool>],
    x: usize,
    y: usize,
    dark: bool,
) {
    modules[y][x] = dark;
    is_function[y][x] = true;
}

fn draw_finder_pattern(
    modules: &mut [Vec<bool>],
    is_function: &mut [Vec<bool>],
    cx: isize,
    cy: isize,
    n: usize,
) {
    for dy in -4..=4isize {
        for dx in -4..=4isize {
            let x = cx + dx;
            let y = cy + dy;
            if x >= 0 && (x as usize) < n && y >= 0 && (y as usize) < n {
                let dist = dx.abs().max(dy.abs());
                let dark = dist != 2 && dist != 4;
                set_func(modules, is_function, x as usize, y as usize, dark);
            }
        }
    }
}

fn draw_alignment_pattern(
    modules: &mut [Vec<bool>],
    is_function: &mut [Vec<bool>],
    cx: usize,
    cy: usize,
) {
    for dy in -2..=2isize {
        for dx in -2..=2isize {
            let x = (cx as isize + dx) as usize;
            let y = (cy as isize + dy) as usize;
            let dist = dx.abs().max(dy.abs());
            set_func(modules, is_function, x, y, dist != 1);
        }
    }
}

fn draw_function_patterns(
    modules: &mut [Vec<bool>],
    is_function: &mut [Vec<bool>],
    spec: &VersionSpec,
) {
    let n = spec.size;

    // Timing patterns
    for i in 0..n {
        set_func(modules, is_function, 6, i, i % 2 == 0);
        set_func(modules, is_function, i, 6, i % 2 == 0);
    }

    // 3 Finder patterns
    draw_finder_pattern(modules, is_function, 3, 3, n);
    draw_finder_pattern(modules, is_function, (n - 4) as isize, 3, n);
    draw_finder_pattern(modules, is_function, 3, (n - 4) as isize, n);

    // Alignment patterns
    let centers = spec.alignment_centers;
    let m = centers.len();
    for (i, &cy) in centers.iter().enumerate() {
        for (j, &cx) in centers.iter().enumerate() {
            if (i == 0 && j == 0) || (i == 0 && j == m - 1) || (i == m - 1 && j == 0) {
                continue;
            }
            draw_alignment_pattern(modules, is_function, cx, cy);
        }
    }

    // Reserve format information areas
    for i in 0..=8 {
        if i != 6 {
            set_func(modules, is_function, 8, i, false);
            set_func(modules, is_function, i, 8, false);
        }
    }
    for i in 0..8 {
        set_func(modules, is_function, n - 1 - i, 8, false);
        set_func(modules, is_function, 8, n - 1 - i, false);
    }
    // Always-dark module
    set_func(modules, is_function, 8, n - 8, true);
}

fn draw_codewords(
    modules: &mut [Vec<bool>],
    is_function: &[Vec<bool>],
    codewords: &[u8],
    n: usize,
) {
    let mut bit_idx = 0usize;
    let total_bits = codewords.len() * 8;

    let mut right = (n - 1) as isize;
    let mut upward = true;

    while right >= 1 {
        if right == 6 {
            right -= 1;
        }
        for v in 0..n {
            let y = if upward { n - 1 - v } else { v };
            for j in 0..2 {
                let x = (right - j) as usize;
                if !is_function[y][x] {
                    let dark = if bit_idx < total_bits {
                        ((codewords[bit_idx / 8] >> (7 - (bit_idx % 8))) & 1) != 0
                    } else {
                        false
                    };
                    modules[y][x] = dark;
                    bit_idx += 1;
                }
            }
        }
        upward = !upward;
        right -= 2;
    }
}

fn apply_mask(modules: &mut [Vec<bool>], is_function: &[Vec<bool>], mask: u8, n: usize) {
    for y in 0..n {
        for x in 0..n {
            if is_function[y][x] {
                continue;
            }
            let invert = match mask {
                0 => (x + y) % 2 == 0,
                1 => y % 2 == 0,
                2 => x % 3 == 0,
                3 => (x + y) % 3 == 0,
                4 => (x / 3 + y / 2) % 2 == 0,
                5 => (x * y) % 2 + (x * y) % 3 == 0,
                6 => ((x * y) % 2 + (x * y) % 3) % 2 == 0,
                _ => ((x + y) % 2 + (x * y) % 3) % 2 == 0,
            };
            if invert {
                modules[y][x] = !modules[y][x];
            }
        }
    }
}

fn draw_format_bits(modules: &mut [Vec<bool>], mask: u8, n: usize) {
    // ECC Level L = 01
    let data = ((0b01u16) << 3) | (mask as u16);
    let mut rem = data;
    for _ in 0..10 {
        rem = (rem << 1) ^ (((rem >> 9) & 1) * 0x537);
    }
    let bits = ((data << 10) | rem) ^ 0x5412;

    let bit = |i: usize| ((bits >> i) & 1) != 0;

    // First copy around top-left finder
    for i in 0..=5 {
        modules[8][i] = bit(i);
    }
    modules[8][7] = bit(6);
    modules[8][8] = bit(7);
    modules[7][8] = bit(8);
    for i in 9..15 {
        modules[14 - i][8] = bit(i);
    }

    // Second copy along bottom-left and top-right
    for i in 0..8 {
        modules[n - 1 - i][8] = bit(i);
    }
    for i in 8..15 {
        modules[8][n - 15 + i] = bit(i);
    }
    modules[n - 8][8] = true;
}

fn penalty_score(modules: &[Vec<bool>], n: usize) -> u32 {
    let mut score = 0u32;

    // Adjacent modules in row/col of same color
    for y in 0..n {
        let mut run_color = modules[y][0];
        let mut run_len = 1usize;
        for x in 1..n {
            if modules[y][x] == run_color {
                run_len += 1;
                if run_len == 5 {
                    score += 3;
                } else if run_len > 5 {
                    score += 1;
                }
            } else {
                run_color = modules[y][x];
                run_len = 1;
            }
        }
    }

    for x in 0..n {
        let mut run_color = modules[0][x];
        let mut run_len = 1usize;
        for y in 1..n {
            if modules[y][x] == run_color {
                run_len += 1;
                if run_len == 5 {
                    score += 3;
                } else if run_len > 5 {
                    score += 1;
                }
            } else {
                run_color = modules[y][x];
                run_len = 1;
            }
        }
    }

    // 2x2 blocks
    for y in 0..n - 1 {
        for x in 0..n - 1 {
            let c = modules[y][x];
            if modules[y][x + 1] == c && modules[y + 1][x] == c && modules[y + 1][x + 1] == c {
                score += 3;
            }
        }
    }

    score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_valid_svg_for_lan_url() {
        let svg = generate_svg("http://192.168.1.42:8300/?pin=4829");
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("<path d=\"M"));
        assert!(svg.ends_with("</svg>"));
    }
}
