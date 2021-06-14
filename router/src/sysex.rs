use midi::{Packet, Message};
use alloc::vec::Vec;

use midi::Message::{SysexEnd2, SysexEnd1, SysexEnd, SysexBegin, SysexCont, SysexEmpty, SysexSingleByte};

use core::convert::TryFrom;
use heapless::spsc::Queue;
use alloc::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum Tag {
    Channel,
    Velocity,
    DeviceId,
    /// Parameter code (cutoff, delay, etc.)
    ParamId,
    /// Control code (knob, pad, etc.)
    ControlId,
    /// Value of parameter
    ValueU7,
    /// Value of parameter
    MsbValueU4,
    /// Value of parameter
    LsbValueU4,
    /// Raw data
    Dump(usize),
}

impl Tag {
    pub fn size(&self) -> usize {
        match self {
            Tag::Dump(len) => *len,
            _ => 1,
        }
    }
}

/// Used to send sysex
/// Accepts same Token as matcher for convenience, but only Match and Val value are sent
// #[derive(Debug)]
pub struct Sysex {
    tokens: Vec<Token>,
    // current token to produce from
    tok_idx: usize,
    // current index inside token
    byte_idx: usize,
    window: Queue<u8, 64>,
}

impl Sysex {
    pub fn new(tokens: Vec<Token>) -> Self {
        Sysex {
            tokens/*: buffer*/,
            tok_idx: 0,
            byte_idx: 0,
            window: Queue::new(),
        }
    }
}

impl Iterator for Sysex {
    type Item = Packet;

    fn next(&mut self) -> Option<Self::Item> {
        if self.tok_idx > self.tokens.len() {
            // final packet already generated
            return None;
        }
        let start = self.tok_idx == 0 && self.byte_idx == 0;
        while self.window.len() < 3 {
            if self.tok_idx >= self.tokens.len() {
                break;
            }
            match &self.tokens[self.tok_idx] {
                Token::Seq(slice) => {
                    if let Err(_) = self.window.enqueue(slice[self.byte_idx]) {
                        break;
                    }
                    self.byte_idx += 1;
                    if self.byte_idx == slice.len() {
                        self.tok_idx += 1;
                        self.byte_idx = 0;
                    }
                }
                Token::Val(val) => {
                    if let Err(_) = self.window.enqueue(*val) {
                        break;
                    }
                    self.tok_idx += 1;
                }
                _ => {}
            };
        }
        if !start && self.window.len() < 3 {
            // mark as done
            self.tok_idx += 1;
        }
        Some(Packet::from(
            match (start, self.window.len()) {
                (true, 0) => SysexEmpty,
                (true, 1) => SysexSingleByte(self.window.dequeue().unwrap()),
                (true, _) => SysexBegin(self.window.dequeue().unwrap(), self.window.dequeue().unwrap()),

                (false, 0) => SysexEnd,
                (false, 1) => SysexEnd1(self.window.dequeue().unwrap()),
                (false, 2) => SysexEnd2(self.window.dequeue().unwrap(), self.window.dequeue().unwrap()),

                (false, _) => SysexCont(self.window.dequeue().unwrap(), self.window.dequeue().unwrap(), self.window.dequeue().unwrap()),
            }
        ))
    }
}

#[allow(unused)]
#[derive(Debug, Clone)]
pub enum Token {
    Seq(&'static [u8]),
    Buf(Vec<u8>),
    Skip(usize),
    Val(u8),
    Cap(Tag),
}

pub type CaptureBuffer = BTreeMap<Tag, Vec<u8>>;

#[derive(Debug)]
pub struct Matcher {
    pattern: Vec<Token>,
    matching: bool,
    // current token to produce from
    tok_idx: usize,
    // current index inside token
    byte_idx: usize,
    captured: CaptureBuffer,
}

impl Matcher {
    pub fn new(pattern: Vec<Token>) -> Self {
        Matcher {
            pattern,
            matching: false,
            tok_idx: 0,
            byte_idx: 0,
            captured: CaptureBuffer::default(),
        }
    }

    pub fn match_packet(&mut self, packet: Packet) -> Option<CaptureBuffer> {
        if let Ok(message) = Message::try_from(packet) {
            let mut sysex_end = true;
            match message {
                SysexBegin(byte0, byte1) => {
                    self.begin_match();
                    self.matching = self.advance(byte0) && self.advance(byte1);
                    sysex_end = false;
                }
                SysexSingleByte(byte0) => {
                    self.begin_match();
                    self.matching = self.advance(byte0);
                }
                SysexEmpty => {
                    self.begin_match();
                    self.matching = true;
                }
                SysexCont(byte0, byte1, byte2) => {
                    self.matching &= self.advance(byte0) && self.advance(byte1) && self.advance(byte2);
                    sysex_end = false;
                }
                SysexEnd => {}
                SysexEnd1(byte0) => self.matching &= self.advance(byte0),
                SysexEnd2(byte0, byte1) => self.matching &= self.advance(byte0) && self.advance(byte1),
                _ => self.matching = false,
            }

            if self.matching & sysex_end {
                self.matching = false;
                return Some(self.captured.clone());
            }
        }
        None
    }

    fn begin_match(&mut self) {
        self.tok_idx = 0;
        self.byte_idx = 0;
        self.captured.clear();
    }

    /// Returns true if byte matched the pattern or was captured
    /// Returns false if byte diverges from pattern
    /// Once this method returns false, every subsequent invocation will also return false until a new Sysex message starts
    fn advance(&mut self, byte: u8) -> bool {
        // fast exit if match previously failed
        if self.tok_idx >= self.pattern.len() {
            return false;
        }
        let mut tok_len = 1;
        match &mut self.pattern[self.tok_idx] {
            Token::Seq(token) => {
                if token[self.byte_idx] != byte {
                    return self.fail_match();
                }
                tok_len = token.len()
            }
            Token::Skip(len) => {
                tok_len = *len as usize
            }
            Token::Val(token) => {
                if *token != byte {
                    return self.fail_match();
                }
            }
            Token::Cap(tag) => {
                self.captured.entry(*tag)
                    .or_insert_with(|| Vec::with_capacity(tag.size()))
                    .push(byte);
                tok_len = tag.size()
            }
            Token::Buf(_) => {}
        };
        self.byte_idx += 1;
        if self.byte_idx >= tok_len {
            // move on to next token
            self.tok_idx += 1;
            self.byte_idx = 0;
        }
        true
    }

    #[inline]
    fn fail_match(&mut self) -> bool {
        self.tok_idx = self.pattern.len();
        false
    }
}
