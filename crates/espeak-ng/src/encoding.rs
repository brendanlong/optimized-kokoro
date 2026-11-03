//! encoding.c's UTF-8 string decoder, the only one `espeak_TextToPhonemes` uses here.

/// `espeak_ng_TEXT_DECODER` reading a UTF-8 C string. As in C, the terminating NUL is part of
/// the input (`length = strlen(string) + 1`), so it is returned as a final 0 character.
#[derive(Debug, Default, Clone)]
pub struct TextDecoder {
    buf: Vec<u8>,
    current: usize,
}

impl TextDecoder {
    /// `text_decoder_decode_string(decoder, string, -1, ESPEAKNG_ENCODING_UTF_8)`. `text` stops at
    /// its first NUL, like a C string.
    pub fn decode_string(&mut self, text: &[u8]) {
        let len = text.iter().position(|&c| c == 0).unwrap_or(text.len());
        self.buf.clear();
        self.buf.extend_from_slice(&text[..len]);
        self.buf.push(0);
        self.current = 0;
    }

    fn end(&self) -> usize {
        self.buf.len()
    }

    pub fn eof(&self) -> bool {
        self.current >= self.end()
    }

    fn next(&mut self) -> u8 {
        let c = self.buf.get(self.current).copied().unwrap_or(0);
        self.current += 1;
        c
    }

    /// `string_decoder_getc_utf_8`.
    pub fn getc(&mut self) -> i32 {
        if self.eof() {
            // C reads past the end here; nothing in the clause reader does that.
            return 0;
        }
        let c = self.next();
        let mut ret: u32;
        match c & 0xf0 {
            0x00..=0x70 => c as i32,
            0x80..=0xb0 => 0xfffd,
            0xc0 | 0xd0 => {
                if self.current + 1 >= self.end() {
                    self.current = self.end();
                    return 0xfffd;
                }
                ret = (c & 0x1f) as u32;
                let c = self.next();
                if c & 0xc0 != 0x80 {
                    self.current -= 1;
                    return 0xfffd;
                }
                ret = (ret << 6) + (c & 0x3f) as u32;
                ret as i32
            }
            0xe0 => {
                if self.current + 2 >= self.end() {
                    self.current = self.end();
                    return 0xfffd;
                }
                ret = (c & 0x0f) as u32;
                for _ in 0..2 {
                    let c = self.next();
                    if c & 0xc0 != 0x80 {
                        self.current -= 1;
                        return 0xfffd;
                    }
                    ret = (ret << 6) + (c & 0x3f) as u32;
                }
                if ret == 0xfffd {
                    return 0x001a;
                }
                ret as i32
            }
            _ => {
                if self.current + 3 >= self.end() {
                    self.current = self.end();
                    return 0xfffd;
                }
                ret = (c & 0x0f) as u32;
                for _ in 0..3 {
                    let c = self.next();
                    if c & 0xc0 != 0x80 {
                        self.current -= 1;
                        return 0xfffd;
                    }
                    ret = (ret << 6) + (c & 0x3f) as u32;
                }
                if ret <= 0x10ffff { ret as i32 } else { 0xfffd }
            }
        }
    }

    pub fn peekc(&mut self) -> i32 {
        if self.eof() {
            return 0;
        }
        let current = self.current;
        let c = self.getc();
        self.current = current;
        c
    }

    /// `text_decoder_get_buffer`: the text not yet read, or `None` at the end.
    pub fn remaining(&self) -> Option<&[u8]> {
        if self.eof() {
            return None;
        }
        Some(&self.buf[self.current..self.buf.len() - 1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_with_terminator() {
        let mut d = TextDecoder::default();
        d.decode_string("aé\u{20ac}".as_bytes());
        assert_eq!([d.getc(), d.getc(), d.getc(), d.getc()], ['a' as i32, 0xe9, 0x20ac, 0]);
        assert!(d.eof());
        d.decode_string(b"\xe2\x82");
        assert_eq!(d.getc(), 0xfffd);
        assert!(d.eof());
        d.decode_string(b"\xc3x");
        assert_eq!([d.getc(), d.getc()], [0xfffd, 'x' as i32]);
    }
}
