//! Latency stamp: a row of black/white blocks at the top of every frame that
//! encodes the host's wall clock at capture time. The browser reads it back
//! from the decoded video and subtracts it from its own clock, which gives the
//! real capture-to-screen latency (exact when host and browser share a clock,
//! as on one machine; phase 2 adds clock sync for two machines).
//!
//! Layout: `BLOCKS` blocks across, each `block = width / BLOCKS` pixels square.
//!   block 0      white  (start marker)
//!   block 1      black  (start marker)
//!   blocks 2..50 48 data bits, MSB first: Unix time in units of 0.1 ms
//!   blocks 50..58 8-bit check: XOR of the 6 data bytes

pub const BLOCKS: usize = 64;
const DATA_BITS: usize = 48;
const USED: usize = 2 + DATA_BITS + 8;
const WHITE_Y: u8 = 235;
const BLACK_Y: u8 = 16;

/// Unix time in 0.1 ms units (fits 48 bits until year ~2059).
pub fn now_tenth_ms() -> u64 {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    (d.as_micros() / 100) as u64
}

fn bits(value: u64) -> [bool; USED] {
    let v = value & ((1u64 << DATA_BITS) - 1);
    let bytes = v.to_be_bytes(); // 8 bytes, top 2 are zero
    let check = bytes[2..].iter().fold(0u8, |a, b| a ^ b);
    let mut out = [false; USED];
    out[0] = true;
    out[1] = false;
    for i in 0..DATA_BITS {
        out[2 + i] = (v >> (DATA_BITS - 1 - i)) & 1 == 1;
    }
    for i in 0..8 {
        out[2 + DATA_BITS + i] = (check >> (7 - i)) & 1 == 1;
    }
    out
}

pub fn block_size(width: usize) -> usize {
    // Even, so the 4:2:0 chroma planes line up with the blocks.
    (width / BLOCKS) & !1
}

/// Write the stamp into an I420 frame (Y, U, V planes, tightly packed).
pub fn write_i420(y: &mut [u8], u: &mut [u8], v: &mut [u8], width: usize, value: u64) {
    let b = block_size(width);
    let cw = width / 2;
    for (i, bit) in bits(value).iter().enumerate() {
        let luma = if *bit { WHITE_Y } else { BLACK_Y };
        let x0 = i * b;
        for row in 0..b {
            y[row * width + x0..row * width + x0 + b].fill(luma);
        }
        for row in 0..b / 2 {
            let s = row * cw + x0 / 2;
            u[s..s + b / 2].fill(128);
            v[s..s + b / 2].fill(128);
        }
    }
}

#[allow(dead_code)] // used by RGBA capture sources (phase 1b)
/// Write the stamp into an RGBA frame (used by sources that capture RGBA).
pub fn write_rgba(rgba: &mut [u8], width: usize, value: u64) {
    let b = block_size(width);
    for (i, bit) in bits(value).iter().enumerate() {
        let c = if *bit { 255 } else { 0 };
        for row in 0..b {
            let start = (row * width + i * b) * 4;
            for px in rgba[start..start + b * 4].chunks_exact_mut(4) {
                px.copy_from_slice(&[c, c, c, 255]);
            }
        }
    }
}

#[allow(dead_code)] // the browser has its own reader; this one is for tests
/// Read the stamp back from a luma-like sampler (`sample(x, y)` → 0..255).
/// Mirrors the browser's reader; used by the tests.
pub fn read(width: usize, sample: impl Fn(usize, usize) -> u8) -> Option<u64> {
    let b = block_size(width);
    let bit = |i: usize| sample(i * b + b / 2, b / 2) > 128;
    if !bit(0) || bit(1) {
        return None;
    }
    let mut v = 0u64;
    for i in 0..DATA_BITS {
        v = (v << 1) | bit(2 + i) as u64;
    }
    let mut check = 0u8;
    for i in 0..8 {
        check = (check << 1) | bit(2 + DATA_BITS + i) as u8;
    }
    let want = v.to_be_bytes()[2..].iter().fold(0u8, |a, b| a ^ b);
    (check == want).then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i420_round_trip() {
        let (w, h) = (1280, 720);
        let mut y = vec![90u8; w * h];
        let mut u = vec![60u8; w * h / 4];
        let mut v = vec![200u8; w * h / 4];
        let value = now_tenth_ms();
        write_i420(&mut y, &mut u, &mut v, w, value);
        assert_eq!(read(w, |x, yy| y[yy * w + x]), Some(value));
    }

    #[test]
    fn rgba_round_trip() {
        let (w, h) = (1920, 1080);
        let mut px = vec![128u8; w * h * 4];
        write_rgba(&mut px, w, 0xABCD_EF01_2345);
        assert_eq!(read(w, |x, yy| px[(yy * w + x) * 4]), Some(0xABCD_EF01_2345));
    }

    #[test]
    fn corrupt_stamp_rejected() {
        let w = 1280;
        let mut y = vec![0u8; w * 64];
        let mut u = vec![0u8; w * 16];
        let mut v = vec![0u8; w * 16];
        write_i420(&mut y, &mut u, &mut v, w, 123_456_789);
        let b = block_size(w);
        // Flip data bit 10.
        let x = (2 + 10) * b + b / 2;
        let flip = if y[(b / 2) * w + x] > 128 { 0 } else { 255 };
        for row in 0..b {
            y[row * w + (2 + 10) * b..row * w + (3 + 10) * b].fill(flip);
        }
        assert_eq!(read(w, |x, yy| y[yy * w + x]), None);
    }
}
