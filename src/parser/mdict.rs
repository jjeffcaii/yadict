use memmap2::Mmap;
use nom::{IResult, bytes::complete::take_till};
use std::cell::OnceCell;
use std::cmp::Ordering;

use super::parser::{BlockEntryInfo, KeyBlock, KeyEntry, record_block_parser};

#[derive(Debug)]
struct RecordOffset {
    buf_offset: usize,
    block_offset: usize,
    record_size: usize,
    decomp_size: usize,
}

pub struct Record<'a> {
    key: &'a [u8],
    mdx: &'a Mdx,
    entry_offset: usize,
    cache: OnceCell<Option<Vec<u8>>>,
}

impl<'a> Record<'a> {
    pub fn key(&self) -> &[u8] {
        self.key
    }

    /// Lazily decompresses and returns the raw definition bytes.
    /// Returns `None` when no record block covers this entry's offset.
    /// The result is cached; repeated calls are free.
    pub fn value(&self) -> Option<&[u8]> {
        self.cache
            .get_or_init(|| self.mdx.fetch_definition(self.entry_offset))
            .as_deref()
    }
}

pub struct Mdx {
    pub(crate) key_blocks: Vec<KeyBlock>,
    pub(crate) records_info: Vec<BlockEntryInfo>,
    pub(crate) mmap: Mmap,
    pub(crate) records_start: usize,
    pub encoding: String,
    pub encrypted: u8,
}

impl Mdx {
    pub fn items(&self) -> impl Iterator<Item = Record<'_>> + '_ {
        self.key_blocks.iter().flat_map(|block| {
            block.entries().map(|entry| Record {
                key: entry.text,
                mdx: self,
                entry_offset: entry.offset,
                cache: OnceCell::new(),
            })
        })
    }

    pub fn keys(&self) -> impl Iterator<Item = KeyEntry<'_>> + '_ {
        self.key_blocks.iter().flat_map(|block| block.entries())
    }

    fn record_offset(&self, entry_offset: usize) -> Option<RecordOffset> {
        let mut block_offset = 0;
        let mut buf_offset = 0;
        for i in &self.records_info {
            if entry_offset <= block_offset + i.decompressed_size {
                return Some(RecordOffset {
                    buf_offset,
                    block_offset: entry_offset - block_offset,
                    record_size: i.compressed_size,
                    decomp_size: i.decompressed_size,
                });
            } else {
                block_offset += i.decompressed_size;
                buf_offset += i.compressed_size;
            }
        }
        None
    }

    pub fn get<A>(&self, key: A) -> Option<Record<'_>>
    where
        A: AsRef<str>,
    {
        let key = key.as_ref().to_lowercase();

        let found = self
            .key_blocks
            .binary_search_by(|probe| {
                let begin = String::from_utf8_lossy(probe.first_key().unwrap()).to_lowercase();
                let end = String::from_utf8_lossy(probe.last_key().unwrap()).to_lowercase();

                if &begin > &key {
                    Ordering::Greater
                } else if &key > &end {
                    Ordering::Less
                } else {
                    Ordering::Equal
                }
            })
            .ok()?;

        let block = &self.key_blocks[found];

        let entry = block.get(&key)?;
        Some(Record {
            key: entry.text,
            mdx: self,
            entry_offset: entry.offset,
            cache: OnceCell::new(),
        })
    }

    fn fetch_definition(&self, entry_offset: usize) -> Option<Vec<u8>> {
        let offset = self.record_offset(entry_offset)?;
        let buf = &self.mmap[self.records_start + offset.buf_offset..];
        let (_, decompressed) =
            record_block_parser(offset.record_size, offset.decomp_size)(buf).unwrap();
        let result: IResult<&[u8], &[u8]> =
            take_till(|x| x == 0)(&decompressed[offset.block_offset..]);
        let (_, raw) = result.unwrap();
        Some(raw.to_vec())
    }
}
