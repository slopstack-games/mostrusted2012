//! MSB-first bit reader and writer (MPEG bitstream order).

pub struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn set_pos(&mut self, pos: usize) {
        self.pos = pos;
    }

    pub fn remaining(&self) -> usize {
        (self.data.len() * 8).saturating_sub(self.pos)
    }

    /// Read up to 32 bits; `None` past the end.
    pub fn get(&mut self, n: u32) -> Option<u32> {
        if n == 0 {
            return Some(0);
        }
        if self.remaining() < n as usize {
            return None;
        }
        let mut v = 0u32;
        for _ in 0..n {
            let byte = self.data[self.pos / 8];
            let bit = (byte >> (7 - self.pos % 8)) & 1;
            v = (v << 1) | bit as u32;
            self.pos += 1;
        }
        Some(v)
    }
}

#[derive(Default)]
pub struct BitWriter {
    pub data: Vec<u8>,
    pos: usize,
}

impl BitWriter {
    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn put(&mut self, n: u32, v: u32) {
        for i in (0..n).rev() {
            if self.pos / 8 >= self.data.len() {
                self.data.push(0);
            }
            if (v >> i) & 1 != 0 {
                self.data[self.pos / 8] |= 0x80 >> (self.pos % 8);
            }
            self.pos += 1;
        }
    }

    /// Copy `n` bits from a reader.
    pub fn copy(&mut self, r: &mut BitReader, mut n: usize) -> Option<()> {
        while n > 0 {
            let k = n.min(32) as u32;
            let v = r.get(k)?;
            self.put(k, v);
            n -= k as usize;
        }
        Some(())
    }

    pub fn align(&mut self) {
        if self.pos % 8 != 0 {
            self.put(8 - (self.pos % 8) as u32, 0);
        }
    }

    /// Zero-pad to `bytes`.
    pub fn pad_to(&mut self, bytes: usize) {
        self.align();
        self.data.resize(bytes.max(self.data.len()), 0);
        self.pos = self.data.len() * 8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut w = BitWriter::default();
        w.put(11, 0x7FF);
        w.put(3, 0b101);
        w.put(32, 0xDEAD_BEEF);
        w.align();
        let mut r = BitReader::new(&w.data);
        assert_eq!(r.get(11), Some(0x7FF));
        assert_eq!(r.get(3), Some(0b101));
        assert_eq!(r.get(32), Some(0xDEAD_BEEF));
    }
}
