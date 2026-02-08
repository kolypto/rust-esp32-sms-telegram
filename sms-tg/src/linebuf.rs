//! Line buffers and friends for realing lines from serial.

use core::ops::Range;

// Ring buffer.
// Actually, a Disruptor: readers may lag behind writers.
//
// One problem here: if the reader is slow, it will get overrun.
pub struct RingBuffer<'a> {
    // The internal memory to manage
    mem: &'a mut [u8],

    // Current write pos
    wpos: usize,

    // Current read pos
    rpos: usize,

    // How many bytes are currently in the buffer?
    // This resolves the ambiguity when `rpos` = `wpos`:
    // - 0 bytes:   everything read =>  the reader just caught up
    // - LEN bytes: not read => the buffer got overrun
    unread: usize,
}

impl <'a> RingBuffer<'a> {
    pub fn new(mem: &'a mut [u8]) -> Self {
        return Self { mem, rpos: 0, wpos: 0, unread: 0 }
    }

    // Reset the buffer.
    pub fn reset(&mut self){
        self.rpos = 0;
        self.wpos = 0;
        self.unread = 0;
    }

    // Get the writable part of the buffer.
    // It will likely be shorter than the original `buf`.
    pub fn writable(&mut self) -> &mut [u8] {
        // rpos < wpos? write until the end
        // rpos > wpos? write until rpos
        // rpos = wpos? ambiguous: all read / overrun
        if self.rpos == self.wpos && self.unread != 0 {
            // OVERRUN. Data loss will occur.
            self.unread = 0;
        }

        if self.rpos <= self.wpos {
            &mut self.mem[self.wpos..]
        } else {
            &mut self.mem[self.wpos..self.rpos]
        }
    }

    // Is the buffer going to be overrun?
    // Check this before writing.
    pub fn is_overrun(&self) -> bool {
        return self.rpos == self.wpos && self.unread != 0;
    }

    // Advance the internal write pos: bytes written into the buffer
    pub fn has_written(&mut self, n: usize){
        // They couldn't have written more bytes than the length of the slice we've returned.
        // We trust the input.
        self.wpos = (self.wpos + n) % self.mem.len();
        self.unread += n;
    }

    // Get the readable part of the buffer with data.
    pub fn readable(&self) -> &[u8] {
        // rpos < wpos? read until wpos
        // rpos > wpos? read until the end
        // rpos = wpos? ambiguous: empty / read all
        if self.rpos == self.wpos && self.unread == 0 {
            &self.mem[0..0] // nothing to read
        } else if self.rpos < self.wpos {
            &self.mem[self.rpos..self.wpos]
        } else {
            &self.mem[self.rpos..]
        }
    }

    // Advance the internal read pos: bytes read from the read buffer
    pub fn has_read(&mut self, n: usize){
        // They couldn't have read more bytes than the length of the slice we're returned.
        // We trust the input.
        self.rpos = (self.rpos + n) % self.mem.len();
        self.unread -= n;
    }

    // Find the ranges for a complete line ending with \n, if any.
    // Returns two ranges: because the ring buffer may wrap around
    fn find_complete_line_range(&self) -> Option<(Range<usize>, Range<usize>)> {
        // Which ranges we can read?
        if self.unread == 0 {
            return None;
        }
        let (first, second) = if self.rpos < self.wpos {
            (self.rpos..self.wpos, 0..0)
        } else {
            (self.rpos..self.mem.len(), 0..self.wpos)
        };

        // TODO: this can be optimized by remembering where we stopped last.

        // Go looking for \n in the first chunk
        if let Some(i) = self.mem[first.clone()].iter().position(|&b| b == b'\n').map(|i| i+first.start) {
           return Some((first.start..i, 0..0));
        }
        // Go looking for \n in the second
        if let Some(i) = self.mem[second.clone()].iter().position(|&b| b == b'\n').map(|i| i + second.start) {
           return Some((first, second.start..i));
        }

        // Nothing found
        None
    }

    // Get complete line\n, if any, into the provided buffer.
    // Remember to do .has_read(n+1) afterwards
    pub fn copy_line_into_slice(&mut self, line: &mut [u8]) -> Option<usize> {
        if let Some((first, second)) = self.find_complete_line_range() {
            // Copy
            let (n1, n2) = (first.len(), second.len());
            line[..n1].copy_from_slice(&self.mem[first.clone()]);
            line[n1..n1+n2].copy_from_slice(&self.mem[second.clone()]);
            Some(first.len() + second.len())
        } else {
            None
        }
    }

    // Read line and advance the pointer
    pub fn read_line<'s>(&mut self, line: &'s mut [u8]) -> Option<&'s [u8]> {
        if let Some(n) = self.copy_line_into_slice(line) {
            self.has_read(n+1);
            Some(&line[..n])
        } else {
            None
        }
    }
}

// Iterator over a buffer: find all lines (i.e. \n-separated)
pub struct LineIterator<'a> {
    // Buffer to read from from
    buf: &'a [u8],

    // Search start from.
    // Also: current parsing position.
    // Initially = 0, unless you're certain that there's no \n up to a certain index: then uses a different value.
    pos: usize,

    // Line stars at, if found.
    // Initially = 0 (the start of the buffer), then goes equal to `pos`
    line_starts_at: usize,
}

impl<'a> LineIterator<'a> {
    pub fn new(buf: &'a [u8], from_pos: usize) -> Self {
        Self{ buf, pos: from_pos, line_starts_at: 0 }
    }
}

impl <'a> Iterator for LineIterator<'a> {
    type Item = Range<usize>;

    fn next(&mut self) -> Option<Range<usize>> {
        let nl = self.buf[self.pos..].iter()
            .position(|&b| b == b'\n').map(|i| i + self.pos);

        // \n found: we've got a line!
        if let Some(i) = nl {
            // Complete line
            let linerange = self.line_starts_at..i;

            // Next
            self.line_starts_at = i+1; // next time, copy from this index
            self.pos = i+1;

            // Return
            return Some(linerange)
        } else {
            return None
        }
    }
}

// doesn't work
#[cfg(test)]
mod tests {
    // Note this useful idiom: importing names from outer (for mod tests) scope.
    use super::*;

    #[test]
    fn test_line_iterator() {
        let buffer = b"hello\n\nworld\nfoo";

        // Lines
        let mut iter = LineIterator::new(buffer, 4);
        assert_eq!(iter.next(), Some(0..5)); // "hello"
        assert_eq!(iter.next(), Some(6..6));
        assert_eq!(iter.next(), Some(7..12)); // "world"
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn test_empty_buffer() {
        let buffer = b"";
        let mut iter = LineIterator::new(buffer, 0);
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn test_ring_buffer(){
        let mut mem = [0_u8; 5];
        let mut rbuf = RingBuffer::new(&mut mem);

        // >> Write 3 bytes
        // [r..w.]
        {
            let buf = rbuf.writable();
            assert_eq!(buf.len(), 5); // the whole mem is available
            buf[0..3].copy_from_slice(&[1, 2, 3]);
            rbuf.has_written(3);
        }
        assert_eq!(rbuf.wpos, 3);
        assert_eq!(rbuf.unread, 3);
        assert_eq!(rbuf.mem, &[1, 2, 3, 0, 0]);

        // >> Write 2 bytes (until the end)
        // [w.r...]
        {
            let buf = rbuf.writable();
            assert_eq!(buf.len(), 2); // only 2 bytes left this time
            buf[0..2].copy_from_slice(&[4, 5]);
            rbuf.has_written(2);
        }
        assert_eq!(rbuf.wpos, 0);
        assert_eq!(rbuf.mem, &[1, 2, 3, 4, 5]);
        assert_eq!(rbuf.unread, 5);

        // >> Read 2 bytes
        // [..r.w.]
        {
            let buf = rbuf.readable();
            assert_eq!(buf, &[1, 2, 3, 4, 5]);
            rbuf.has_read(2);
        }
        assert_eq!(rbuf.rpos, 2);
        assert_eq!(rbuf.unread, 3);

        // >> Write 2 more (catch up with rpos)
        // [..w/r...]
        {
            let buf = rbuf.writable();
            assert_eq!(buf.len(), 2); // until rpos
            buf[0..2].copy_from_slice(&[6, 7]);
            rbuf.has_written(2);
        }
        assert_eq!(rbuf.wpos, 2);
        assert_eq!(rbuf.mem, &[6, 7, 3, 4, 5]);
        assert_eq!(rbuf.unread, 5);

        // ### READER CATCHES UP

        // >> Read until the end (wrapped around)
        // [r.w..]
        {
            let buf = rbuf.readable();
            assert_eq!(buf, &[3, 4, 5]);
            rbuf.has_read(3);
        }
        assert_eq!(rbuf.rpos, 0);
        assert_eq!(rbuf.unread, 2);

        // >> Read until wpos (caught up)
        // [..r/w..]
        {
            let buf = rbuf.readable();
            assert_eq!(buf, &[6 ,7]);
            rbuf.has_read(2);
        }
        assert_eq!(rbuf.rpos, 2);
        assert_eq!(rbuf.unread, 0);

        // >> Read again? nothing to read
        {
            let buf = rbuf.readable();
            assert_eq!(buf.len(), 0); // nothing to read
        }

        // ### WRITER OVERRUN

        // >> Write 3 bytes (wrapped around)
        // [w.r...]
        {
            let buf = rbuf.writable();
            assert_eq!(buf.len(), 3); // until end
            buf[0..3].copy_from_slice(&[9, 9, 9]);
            rbuf.has_written(3);
        }
        assert_eq!(rbuf.wpos, 0);
        assert_eq!(rbuf.mem, &[6, 7, 9, 9, 9]);
        assert_eq!(rbuf.unread, 3);

        // >> Write 2 bytes (caught up with rpos)
        // [..w/r...]
        {
            let buf = rbuf.writable();
            assert_eq!(buf.len(), 2); // until rpos
            buf[0..2].copy_from_slice(&[8, 8]);
            rbuf.has_written(2);
        }
        assert_eq!(rbuf.wpos, 2);
        assert_eq!(rbuf.mem, &[8, 8, 9, 9, 9]);
        assert_eq!(rbuf.unread, 5);

        // Write again. OVERRUN. (wrapped around)
        // [w.r...]
        {
            let buf = rbuf.writable();
            assert_eq!(buf.len(), 3); // until end
            buf[0..3].copy_from_slice(&[7, 7, 7]);
            rbuf.has_written(3);
        }
        assert_eq!(rbuf.wpos, 0);
        assert_eq!(rbuf.mem, &[8, 8, 7, 7, 7]);
        assert_eq!(rbuf.unread, 3);
    }

    #[test]
    fn test_ring_buffer_newlines(){
        let mut mem = [0_u8; 16];
        mem.copy_from_slice("aaa\n\nbbb\nccc\nddd".as_bytes());
        let mut rbuf = RingBuffer::new(&mut mem);
        rbuf.has_written(10); // in the middle of ccc

        // >>> Read: aaa
        let mut linebuf = [0_u8; 16];
        let line = rbuf.read_line(&mut linebuf);
        assert_eq!(line.unwrap(), &[97, 97, 97]);  // aaa

        // >>> Read: ""
        let line = rbuf.read_line(&mut linebuf);
        println!("line: {:?}", line);
        assert_eq!(line.unwrap(), &[]);  // ''

        // >>> Read: bbb
        let line = rbuf.read_line(&mut linebuf);
        assert_eq!(line.unwrap(), &[98, 98, 98]);  // bbb

        // >>> Read: ccc -- can't; it's incomplete
        let line = rbuf.read_line(&mut linebuf);
        assert_eq!(line, None);

        // >>> Write more. Wrap around.
        rbuf.has_written(5);
        rbuf.has_written(7);

        // >>> Read: ccc -- yes
        let line = rbuf.read_line(&mut linebuf);
        assert_eq!(line.unwrap(), &[99, 99, 99]);  // ccc

        // >>> Read: ddd + aaa
        let line = rbuf.read_line(&mut linebuf);
        assert_eq!(line.unwrap(), &[100, 100, 100, 97, 97, 97]);  // ddd + aaa
    }

}
