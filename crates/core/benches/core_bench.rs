use criterion::{black_box, criterion_group, criterion_main, Criterion};
use nw_core::block::{Block, BlockKind};
use nw_core::search::SearchIndex;
use nw_core::sync::merge_pages;
use nw_core::{MarkKind, Page, RichText};

fn big_page(n: usize) -> Page {
    let mut p = Page::new("bench", None);
    p.blocks = (0..n)
        .map(|i| Block::with_text(BlockKind::Paragraph, &format!("block number {i} with some words to index")))
        .collect();
    p
}

fn editor(c: &mut Criterion) {
    let base = "The quick brown fox jumps over the lazy dog. ".repeat(100);
    let mut rt = RichText::new(&base);
    rt.toggle(10, 400, MarkKind::Bold);
    c.bench_function("richtext_set_plain_4_5k_chars", |b| {
        let edited = format!("X{base}");
        b.iter(|| {
            let mut t = rt.clone();
            t.set_plain(black_box(&edited));
            t
        });
    });
    c.bench_function("richtext_segments", |b| b.iter(|| black_box(rt.segments())));
}

fn search(c: &mut Criterion) {
    let page = big_page(10_000);
    c.bench_function("index_page_10k_blocks", |b| {
        b.iter(|| {
            let mut idx = SearchIndex::new();
            idx.index_page(&page);
            idx
        });
    });
    let mut idx = SearchIndex::new();
    for _ in 0..200 {
        idx.index_page(&big_page(20));
    }
    c.bench_function("query_200_pages", |b| b.iter(|| black_box(idx.query("block words"))));
}

fn sync(c: &mut Criterion) {
    let a = big_page(5_000);
    let mut b = a.clone();
    b.updated += 1;
    b.blocks.push(Block::with_text(BlockKind::Paragraph, "new"));
    c.bench_function("merge_5k_block_pages", |bch| bch.iter(|| black_box(merge_pages(&a, &b))));
}

criterion_group!(benches, editor, search, sync);
criterion_main!(benches);
